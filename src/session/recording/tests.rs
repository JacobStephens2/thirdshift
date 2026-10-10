use super::*;
use crate::harness::Harness;

struct Fixture {
    root: tempfile::TempDir,
    recording: Recording,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let recording = Recording::new(Some(root.path().join("7-stamp.log")), None, "stamp".into());
        Self { root, recording }
    }

    fn log(&self, name: &str) -> PathBuf {
        self.root.path().join(format!("7-stamp-{name}.jsonl"))
    }

    fn run(&self, name: &str, script: &str, choice: &Choice) -> Result<Ended> {
        self.recording.run(
            name,
            choice,
            Command::new("/bin/sh").args(["-c", script]),
            None,
            &self.log(name),
            choice
                .harness
                .adapter()
                .interpretation(self.root.path(), ""),
        )
    }
}

const MODEL: &str = r#"{"type":"assistant","message":{"model":"reported-model","content":[]}}"#;

#[test]
fn execution_failure_keeps_observed_models_for_the_owning_command() {
    let fixture = Fixture::new();
    let error = fixture
        .run(
            "implement",
            &format!("echo '{MODEL}'; exit 3"),
            &Choice::default(),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "claude exited 3");
    assert_eq!(
        fixture.recording.notification_lines(),
        ["- implement: claude · reported-model · session effort: default effort"]
    );
}

const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;

#[test]
fn retained_models_replace_stream_evidence_including_an_empty_authoritative_set() {
    for (retained, expected) in [
        (
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"retained-model"}}}"#,
            "retained-model",
        ),
        (
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"assistant_message_committed","text":"retained reply"}}}"#,
            "requested-model (requested)",
        ),
    ] {
        let fixture = Fixture::new();
        let dir = fixture.root.path().join("sessions/s1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session.jsonl"), retained).unwrap();
        let choice = Choice {
            model: Some("requested-model".into()),
            ..Choice::default()
        };
        let ended = fixture
            .recording
            .run(
                "implement",
                &choice,
                Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                None,
                &fixture.log("implement"),
                crate::harness::interpretation_tests::recording_interpretation(
                    fixture.root.path(),
                    Harness::Muse,
                ),
            )
            .unwrap();
        assert!(ended.killed.is_empty());
        assert_eq!(
            fixture.recording.notification_lines(),
            [format!(
                "- implement: claude · {expected} · session effort: default effort"
            )]
        );
    }
}

/// Optional export uses PATH; run just this test in a process whose PATH
/// contains our stand-in, keeping both signal and environment state isolated.
fn with_export(test: &str, script: &str, check: impl FnOnce(&Path)) {
    const ROOT: &str = "THIRDSHIFT_RECORDING_EXPORT";
    if let Some(root) = std::env::var_os(ROOT) {
        check(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("bin")).unwrap();
    crate::test_support::write_executable(&root.path().join("bin/opencode"), script);
    let path = std::env::join_paths(
        std::iter::once(root.path().join("bin"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("session::recording::tests::{test}"),
            "--nocapture",
        ])
        .env(ROOT, root.path())
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn absent_completion_report_preserves_the_stream_checkpoint() {
    with_export(
        "absent_completion_report_preserves_the_stream_checkpoint",
        r#"#!/bin/sh
cp "$THIRDSHIFT_RECORDING_EXPORT/7-stamp-implement.models.json" "$THIRDSHIFT_RECORDING_EXPORT/checkpoint.json"
kill -INT "$PPID"
sleep 1
"#,
        |root| {
            crate::interrupt::install().unwrap();
            let recording = Recording::new(Some(root.join("7-stamp.log")), None, "stamp".into());
            let log = root.join("7-stamp-implement.jsonl");
            let error = recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                    None,
                    &log,
                    crate::harness::interpretation_tests::recording_interpretation(
                        root,
                        Harness::OpenCode,
                    ),
                )
                .unwrap_err();
            assert_eq!(error.to_string(), "interrupted");
            assert_eq!(
                fs::read(log.with_extension("models.json")).unwrap(),
                fs::read(root.join("checkpoint.json")).unwrap()
            );
            assert_eq!(
                recording.notification_lines(),
                ["- implement: claude · reported-model · session effort: default effort"]
            );
        },
    );
}

#[test]
fn both_evidence_writes_use_the_captured_owning_command() {
    with_export(
        "both_evidence_writes_use_the_captured_owning_command",
        r#"#!/bin/sh
cp "$THIRDSHIFT_RECORDING_EXPORT/7-stamp-implement.models.json" "$THIRDSHIFT_RECORDING_EXPORT/checkpoint.json"
echo '{"info":{"outcome":"completed"},"messages":[{"type":"assistant","model":{"providerID":"provider","id":"retained-model"},"content":[]}]}'
"#,
        |root| {
            let owner = root.join("7-stamp.log");
            let recording = Recording::new(
                Some(owner.clone()),
                Some(root.join("other.log")),
                "stamp".into(),
            );
            let log = root.join("7-stamp-implement.jsonl");
            recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                    None,
                    &log,
                    crate::harness::interpretation_tests::recording_interpretation(
                        root,
                        Harness::OpenCode,
                    ),
                )
                .unwrap();
            // Persisted identity is part of the existing record compatibility contract.
            for path in [
                root.join("checkpoint.json"),
                log.with_extension("models.json"),
            ] {
                let json: serde_json::Value =
                    serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
                assert_eq!(json["command_log"], owner.to_str().unwrap());
            }
            assert_eq!(
                recording.notification_lines(),
                ["- implement: claude · provider/retained-model · session effort: default effort"]
            );
        },
    );
}

#[test]
fn reader_failure_keeps_models_consumed_before_the_log_failed() {
    let fixture = Fixture::new();
    let log = fixture.log("implement");
    assert!(Command::new("mkfifo").arg(&log).status().unwrap().success());
    let released = fixture.root.path().join("released");
    let reader_release = released.clone();
    let reader_log = log.clone();
    let reader = std::thread::spawn(move || {
        let mut input = BufReader::new(File::open(reader_log).unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(input.read_line(&mut line).unwrap(), 0);
            if line == "ready\n" {
                break;
            }
        }
        drop(input);
        fs::write(reader_release, "").unwrap();
    });
    let script = format!(
        "echo '{MODEL}'; echo ready; while ! test -f '{}'; do sleep 0.01; done; echo fail",
        released.display()
    );
    let error = fixture
        .recording
        .run(
            "implement",
            &Choice::default(),
            Command::new("/bin/sh").args(["-c", &script]),
            None,
            &log,
            Harness::Claude
                .adapter()
                .interpretation(fixture.root.path(), ""),
        )
        .unwrap_err();
    reader.join().unwrap();
    assert!(error.to_string().contains("could not write"), "{error:#}");
    assert_eq!(
        fixture.recording.notification_lines(),
        ["- implement: claude · reported-model · session effort: default effort"]
    );
}

#[test]
fn collection_preserves_legacy_selection_order_deduplication_and_fallbacks() {
    let fixture = Fixture::new();
    let owner = fixture.root.path().join("7-stamp.log");
    for (file, command, models, requested, effort) in [
        (
            "9-stamp-implement",
            Some(owner.clone()),
            vec!["a", "b", "a"],
            None,
            None,
        ),
        (
            "8-stamp-review",
            Some(owner.clone()),
            vec![],
            Some("requested"),
            Some("high"),
        ),
        ("7-stamp-implement", Some(owner.clone()), vec![], None, None),
        (
            "6-stamp-implement",
            Some(fixture.root.path().join("other.log")),
            vec!["wrong-owner"],
            None,
            None,
        ),
        ("5-stamp-implement", None, vec!["unowned"], None, None),
        (
            "4-old-implement",
            Some(owner),
            vec!["old-stamp"],
            None,
            None,
        ),
    ] {
        let record = serde_json::json!({
            "command_log": command, "kind": if file.contains("review") { "review" } else { "implement" },
            "harness": "claude", "requested": requested, "effort": effort, "observed": models,
        });
        fs::write(
            fixture.root.path().join(format!("{file}.models.json")),
            record.to_string(),
        )
        .unwrap();
    }
    fs::write(
        fixture.root.path().join("0-stamp-implement.models.json"),
        "{ corrupt",
    )
    .unwrap();
    assert_eq!(
        fixture.recording.notification_lines(),
        [
            "- implement: claude · Harness default (not reported) · session effort: default effort",
            "- review: claude · requested (requested) · session effort: high",
            "- implement: claude · a, b · session effort: default effort",
        ]
    );
}

#[test]
fn evidence_write_and_directory_read_failures_do_not_change_session_outcomes() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.log("implement").with_extension("models.json")).unwrap();
    assert!(
        fixture
            .run("implement", &format!("echo '{MODEL}'"), &Choice::default())
            .is_ok()
    );
    assert!(fixture.recording.notification_lines().is_empty());
    let missing = Recording::new(
        Some(fixture.root.path().join("missing/command.log")),
        None,
        "stamp".into(),
    );
    assert!(missing.notification_lines().is_empty());
}

#[test]
fn absent_ownership_removes_stale_child_environment_and_records_no_other_command() {
    let fixture = Fixture::new();
    let unowned = Recording::new(None, None, "stamp".into());
    let mut child = Command::new("/bin/sh");
    child
        .args(["-c", "test -z \"${THIRDSHIFT_PARENT_COMMAND_LOG+x}\""])
        .env(PARENT_COMMAND_LOG, "stale.log");
    unowned.inherit_command(&mut child);
    assert!(child.status().unwrap().success());
    unowned
        .run(
            "implement",
            &Choice::default(),
            Command::new("/bin/sh").args(["-c", &format!("echo '{MODEL}'")]),
            None,
            &fixture.log("implement"),
            Harness::Claude
                .adapter()
                .interpretation(fixture.root.path(), ""),
        )
        .unwrap();
    assert!(unowned.notification_lines().is_empty());
    assert!(fixture.recording.notification_lines().is_empty());
}

#[test]
fn nested_children_keep_the_exact_owning_command() {
    const CHILD: &str = "THIRDSHIFT_RECORDING_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = Path::new(&root);
        let depth: u8 = std::env::var("THIRDSHIFT_RECORDING_DEPTH")
            .unwrap()
            .parse()
            .unwrap();
        logs::begin(logs::Begin::ChildRun("stamp", logs::CommandKind::Issue));
        let recording = Recording::capture();
        recording
            .run(
                "implement",
                &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!("echo '{MODEL}'")]),
                None,
                &root.join(format!("{depth}-stamp-implement.jsonl")),
                Harness::Claude.adapter().interpretation(root, ""),
            )
            .unwrap();
        if depth < 9 {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "session::recording::tests::nested_children_keep_the_exact_owning_command",
                ])
                .env("THIRDSHIFT_RECORDING_DEPTH", (depth + 1).to_string());
            recording.inherit_command(&mut child);
            assert!(child.status().unwrap().success());
        }
        return;
    }
    let fixture = Fixture::new();
    // A local Command wins even when the supplied inherited owner differs.
    let recording = Recording::new(
        Some(fixture.root.path().join("7-stamp.log")),
        Some(fixture.root.path().join("wrong.log")),
        "stamp".into(),
    );
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "session::recording::tests::nested_children_keep_the_exact_owning_command",
        ])
        .env(CHILD, fixture.root.path())
        .env("THIRDSHIFT_RECORDING_DEPTH", "8")
        .env(PARENT_COMMAND_LOG, "stale.log");
    recording.inherit_command(&mut child);
    let output = child.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        recording.notification_lines(),
        [
            "- implement: claude · reported-model · session effort: default effort",
            "- implement: claude · reported-model · session effort: default effort",
        ]
    );
}

#[test]
fn interrupted_stream_survives_process_owners_suppression_of_recovered_state() {
    with_export(
        "interrupted_stream_survives_process_owners_suppression_of_recovered_state",
        "#!/bin/sh\nexit 99\n",
        |root| {
            crate::interrupt::install().unwrap();
            let recording = Recording::new(Some(root.join("7-stamp.log")), None, "stamp".into());
            let error = recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args([
                        "-c",
                        &format!("echo '{MODEL}'; kill -INT \"$PPID\"; sleep 5"),
                    ]),
                    None,
                    &root.join("7-stamp-implement.jsonl"),
                    Harness::Claude.adapter().interpretation(root, ""),
                )
                .unwrap_err();
            assert_eq!(error.to_string(), "interrupted");
            assert_eq!(
                recording.notification_lines(),
                ["- implement: claude · reported-model · session effort: default effort"]
            );
        },
    );
}

#[test]
fn startup_failure_keeps_its_cause_without_writing_evidence() {
    let fixture = Fixture::new();
    let error = fixture
        .recording
        .run(
            "implement",
            &Choice::default(),
            &mut Command::new(fixture.root.path().join("missing-cli")),
            None,
            &fixture.log("implement"),
            Harness::Claude
                .adapter()
                .interpretation(fixture.root.path(), ""),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "could not run claude");
    assert!(
        !fixture
            .log("implement")
            .with_extension("models.json")
            .exists()
    );
    assert!(fixture.recording.notification_lines().is_empty());
}
#[test]
fn completion_progress_comes_from_interpretation_without_duplicate_live_lines() {
    const CHILD: &str = "THIRDSHIFT_RECORDING_PROGRESS";
    if std::env::var_os(CHILD).is_some() {
        let fixture = Fixture::new();
        let dir = fixture.root.path().join("sessions/s1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("session.jsonl"), [
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"first"}}}"#,
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"second"}}}"#,
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"first"}}}"#,
        ].join("\n")).unwrap();
        // Choice controls execution and metadata; Interpretation supplies reporting.
        fixture
            .recording
            .run(
                "retained",
                &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                None,
                &fixture.log("retained"),
                crate::harness::interpretation_tests::recording_interpretation(
                    fixture.root.path(),
                    Harness::Muse,
                ),
            )
            .unwrap();
        fs::write(dir.join("session.jsonl"), "").unwrap();
        fixture
            .recording
            .run(
                "empty",
                &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                None,
                &fixture.log("empty"),
                crate::harness::interpretation_tests::recording_interpretation(
                    fixture.root.path(),
                    Harness::Muse,
                ),
            )
            .unwrap();
        fixture
            .recording
            .run(
                "live",
                &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!("echo '{MODEL}'; echo '{MODEL}'")]),
                None,
                &fixture.log("live"),
                Harness::Claude
                    .adapter()
                    .interpretation(fixture.root.path(), "")
                    .for_security(None),
            )
            .unwrap();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session::recording::tests::completion_progress_comes_from_interpretation_without_duplicate_live_lines", "--nocapture"])
        .env(CHILD, "1").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let models: Vec<_> = stderr
        .lines()
        .filter_map(|line| match progress::ChildLine::of(line) {
            progress::ChildLine::Own(message) if message.contains(": Model:") => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        models,
        [
            "retained: Model: first",
            "retained: Model: second",
            "retained: Model: first",
            "live: Model: reported-model"
        ],
        "{stderr}"
    );
    assert_eq!(stderr.matches("session ended after").count(), 3, "{stderr}");
}
