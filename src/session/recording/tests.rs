use super::*;
use crate::harness::Harness;
use std::fs::File;

const PARENT_OWNER: &str = "THIRDSHIFT_LOG_OWNER";

struct Fixture {
    root: tempfile::TempDir,
    recording: Recording,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let recording = Recording::new(
            Some(root.path().join("7-20261003T120000-0400.log")),
            None,
            "20261003T120000-0400".into(),
        );
        Self { root, recording }
    }

    fn log(&self, name: &str) -> PathBuf {
        self.root
            .path()
            .join(format!("7-20261003T120000-0400-{name}.jsonl"))
    }

    fn run(&self, name: &str, script: &str, choice: &Choice) -> Result<Ended> {
        self.recording.run(
            name,
            choice,
            Command::new("/bin/sh").args(["-c", script]),
            Input::default(),
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
fn private_recording_retains_live_scope_evidence_after_events_disappear() {
    let fixture = Fixture::new();
    let scope = crate::scope::testing::Fixture::new();
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        &format!(
            "echo '{MODEL}'; echo 'oom_kill 1' > \"$EVENTS\"; {}",
            crate::scope::testing::REMOVE_OBSERVED_EVENTS
        ),
    ]);
    let mut launched = scope.launch(command);
    let choice = Choice::default();
    let result = Recording::for_trial().run(
        "model Grader",
        &choice,
        &mut launched,
        Input {
            private: true,
            ..Input::default()
        },
        &fixture.log("grader"),
        choice
            .harness
            .adapter()
            .interpretation(fixture.root.path(), ""),
    );
    assert!(result.is_err());
    assert!(!scope.path("events").exists());
    assert!(launched.oom_killed());
    assert_eq!(scope.journal_reads(), 0);
    let trace = fs::read_to_string(fixture.log("grader").with_extension("logs.jsonl")).unwrap();
    assert!(trace.contains("reported-model"));
}

#[test]
fn all_harnesses_preserve_raw_events_only_in_otlp_session_traces() {
    use crate::otlp_jsonl::{Logs, Reader, Spans};
    use opentelemetry_proto::tonic::collector::{
        logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest,
    };
    for harness in Harness::ALL {
        let fixture = Fixture::new();
        let protocol = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/normalized-{}.jsonl",
            harness.name()
        ));
        let raw = fs::read_to_string(&protocol).unwrap();
        let choice = Choice {
            harness,
            ..Choice::default()
        };
        let log = fixture.log("implement");
        let _ = fixture.recording.run(
            "implement",
            &choice,
            Command::new("cat").arg(&protocol),
            Input {
                prompt: "session prompt",
                ..Default::default()
            },
            &log,
            harness
                .adapter()
                .interpretation(fixture.root.path(), "session prompt"),
        );
        assert!(!log.exists());
        let logs = fs::read_to_string(log.with_extension("logs.jsonl")).unwrap();
        let spans = fs::read_to_string(log.with_extension("spans.jsonl")).unwrap();
        let requests: Vec<_> = Reader::<_, Logs>::new(logs.as_bytes())
            .collect::<std::io::Result<_>>()
            .unwrap();
        assert!(requests.len() >= raw.lines().count());
        for (request, raw) in requests.iter().zip(raw.lines()) {
            let record = &request.resource_logs[0].scope_logs[0].log_records[0];
            assert_eq!(
                record.body,
                Some(crate::otlp_jsonl::AnyValue::String(raw.into()))
            );
            assert!(record.trace_id.is_some());
            assert!(record.span_id.is_some());
            assert!(record.observed_time_unix_nano > 0);
        }
        for line in logs.lines() {
            // Official AnyValue flattens its oneof: parse the arbitrary-precision
            // JSON numbers into Value first, as in the writer's protocol tests.
            let official: ExportLogsServiceRequest =
                serde_json::from_value(serde_json::from_str::<serde_json::Value>(line).unwrap())
                    .unwrap();
            assert_eq!(official.resource_logs[0].scope_logs[0].log_records.len(), 1);
        }
        let requests: Vec<_> = Reader::<_, Spans>::new(spans.as_bytes())
            .collect::<std::io::Result<_>>()
            .unwrap();
        assert!(!requests.is_empty());
        if harness == Harness::Claude {
            let model = requests
                .iter()
                .flat_map(|r| &r.resource_spans)
                .flat_map(|r| &r.scope_spans)
                .flat_map(|s| &s.spans)
                .find(|span| {
                    span.attributes.iter().any(|attr| {
                        attr.key == "gen_ai.operation.name"
                            && attr.value
                                == Some(crate::otlp_jsonl::AnyValue::String("chat".into()))
                    })
                })
                .unwrap();
            assert!(
                model
                    .attributes
                    .iter()
                    .any(|attr| attr.key == "gen_ai.usage.input_tokens"
                        && attr.value == Some(crate::otlp_jsonl::AnyValue::Int(13))),
                "cached tokens are part of GenAI input usage: {model:?}"
            );
        }
        for line in spans.lines() {
            let official: ExportTraceServiceRequest =
                serde_json::from_value(serde_json::from_str::<serde_json::Value>(line).unwrap())
                    .unwrap();
            assert!(!official.resource_spans[0].scope_spans[0].spans.is_empty());
        }
    }
}

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
        ["- implement: claude · reported-model (reported) · session effort: default effort"]
    );
}

const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s1"}"#;

#[test]
fn late_permission_metadata_refines_one_ended_span() {
    let fixture = Fixture::new();
    fixture.run("implement", r#"
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"deny","name":"Bash","input":{"command":"git push"}}]}}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"deny","content":"opaque failure","is_error":true}]}}'
echo '{"type":"result","subtype":"success","permission_denials":[{"tool_use_id":"deny","tool_name":"Bash","tool_input":{"command":"git push"}}]}'
"#, &Choice::default()).unwrap();
    let bytes = fs::read(fixture.log("implement").with_extension("spans.jsonl")).unwrap();
    let tools: Vec<_> =
        crate::otlp_jsonl::Reader::<_, crate::otlp_jsonl::Spans>::new(bytes.as_slice())
            .flat_map(|request| request.unwrap().resource_spans)
            .flat_map(|resource| resource.scope_spans)
            .flat_map(|scope| scope.spans)
            .filter(|span| span.name == "execute_tool Bash")
            .collect();
    assert_eq!(tools.len(), 1);
    assert!(
        tools[0]
            .attributes
            .iter()
            .any(|attr| attr.key == "thirdshift.outcome"
                && attr.value == Some(crate::otlp_jsonl::AnyValue::String("denied".into()))),
        "{tools:?}"
    );
}

#[test]
fn retained_models_replace_stream_evidence_including_an_empty_authoritative_set() {
    for (retained, expected) in [
        (
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"retained-model"}}}"#,
            "retained-model (reported)",
        ),
        (
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"assistant_message_committed","text":"retained reply"}}}"#,
            "requested-model (requested; actual model not reported)",
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
                Input::default(),
                &fixture.log("implement"),
                crate::harness::interpretation_tests::recording_interpretation(
                    fixture.root.path(),
                    Harness::Muse,
                ),
            )
            .unwrap();
        assert!(ended.killed.is_empty());
        let trace =
            fs::read_to_string(fixture.log("implement").with_extension("logs.jsonl")).unwrap();
        assert!(
            crate::otlp_jsonl::Reader::<_, crate::otlp_jsonl::Logs>::new(trace.as_bytes())
                .map(
                    |request| request.unwrap().resource_logs[0].scope_logs[0].log_records[0]
                        .body
                        .clone()
                )
                .any(|body| body == Some(crate::otlp_jsonl::AnyValue::String(retained.into()))),
            "retained Harness event must survive as a raw body"
        );
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
cp "$THIRDSHIFT_RECORDING_EXPORT/7-20261003T120000-0400-implement.models.json" "$THIRDSHIFT_RECORDING_EXPORT/checkpoint.json"
kill -INT "$PPID"
sleep 1
"#,
        |root| {
            crate::interrupt::install().unwrap();
            let recording = Recording::new(
                Some(root.join("7-20261003T120000-0400.log")),
                None,
                "20261003T120000-0400".into(),
            );
            let log = root.join("7-20261003T120000-0400-implement.jsonl");
            let error = recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                    Input::default(),
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
                [
                    "- implement: claude · reported-model (reported) · session effort: default effort"
                ]
            );
        },
    );
}

#[test]
fn both_evidence_writes_use_the_captured_owning_command() {
    with_export(
        "both_evidence_writes_use_the_captured_owning_command",
        r#"#!/bin/sh
cp "$THIRDSHIFT_RECORDING_EXPORT/7-20261003T120000-0400-implement.models.json" "$THIRDSHIFT_RECORDING_EXPORT/checkpoint.json"
echo '{"info":{"outcome":"completed"},"messages":[{"type":"assistant","model":{"providerID":"provider","id":"retained-model"},"content":[]}]}'
"#,
        |root| {
            let owner = root.join("7-20261003T120000-0400.log");
            let recording = Recording::new(
                Some(owner.clone()),
                Some(root.join("other.log")),
                "20261003T120000-0400".into(),
            );
            let log = root.join("7-20261003T120000-0400-implement.jsonl");
            recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'; echo '{MODEL}'")]),
                    Input::default(),
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
                [
                    "- implement: claude · provider/retained-model (reported) · session effort: default effort"
                ]
            );
        },
    );
}

#[test]
fn reader_failure_keeps_models_consumed_before_the_log_failed() {
    let fixture = Fixture::new();
    let log = fixture.log("implement");
    assert!(
        Command::new("mkfifo")
            .arg(log.with_extension("logs.jsonl"))
            .status()
            .unwrap()
            .success()
    );
    let released = fixture.root.path().join("released");
    let reader_release = released.clone();
    let reader_log = log.with_extension("logs.jsonl");
    let reader = std::thread::spawn(move || {
        let mut input = BufReader::new(File::open(reader_log).unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(input.read_line(&mut line).unwrap(), 0);
            if logs::session::events(&line).any(|raw| raw == "ready") {
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
            Input::default(),
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
        ["- implement: claude · reported-model (reported) · session effort: default effort"]
    );
}

#[test]
fn collection_preserves_legacy_selection_order_deduplication_and_fallbacks() {
    let fixture = Fixture::new();
    let owner = fixture.root.path().join("7-20261003T120000-0400.log");
    for (file, command, models, requested, effort) in [
        (
            "9-20261003T120000-0400-implement",
            Some(owner.clone()),
            vec!["a", "b", "a"],
            None,
            None,
        ),
        (
            "8-20261003T120000-0400-review",
            Some(owner.clone()),
            vec![],
            Some("requested"),
            Some("high"),
        ),
        (
            "7-20261003T120000-0400-implement",
            Some(owner.clone()),
            vec![],
            None,
            None,
        ),
        (
            "6-20261003T120000-0400-implement",
            Some(fixture.root.path().join("other.log")),
            vec!["wrong-owner"],
            None,
            None,
        ),
        (
            "5-20261003T120000-0400-implement",
            None,
            vec!["unowned"],
            None,
            None,
        ),
        (
            "4-old-implement",
            Some(owner),
            vec!["old-20261003T120000-0400"],
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
        fixture
            .root
            .path()
            .join("0-20261003T120000-0400-implement.models.json"),
        "{ corrupt",
    )
    .unwrap();
    assert_eq!(
        fixture.recording.notification_lines(),
        [
            "- implement: claude · Harness default (actual model not reported) · session effort: default effort",
            "- review: claude · requested (requested; actual model not reported) · session effort: high",
            "- implement: claude · a, b (reported) · session effort: default effort",
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
        "20261003T120000-0400".into(),
    );
    assert!(missing.notification_lines().is_empty());
}

#[test]
fn absent_ownership_removes_stale_child_environment_and_records_no_other_command() {
    let fixture = Fixture::new();
    let unowned = Recording::new(None, None, "20261003T120000-0400".into());
    let mut child = Command::new("/bin/sh");
    child
        .args(["-c", "test -z \"${THIRDSHIFT_LOG_OWNER+x}\""])
        .env(PARENT_OWNER, "stale.log");
    unowned.inherit_command(&mut child);
    assert!(child.status().unwrap().success());
    unowned
        .run(
            "implement",
            &Choice::default(),
            Command::new("/bin/sh").args(["-c", &format!("echo '{MODEL}'")]),
            Input::default(),
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
        logs::begin(logs::Begin::ChildRun(
            "20261003T120000-0400",
            logs::CommandKind::Issue,
        ));
        let recording = Recording::capture();
        recording
            .run(
                "implement",
                &Choice::default(),
                Command::new("/bin/sh").args(["-c", &format!("echo '{MODEL}'")]),
                Input::default(),
                &recording
                    .session_path(root, &depth.to_string(), "implement")
                    .unwrap(),
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
        Some(
            fixture
                .root
                .path()
                .join("7-20261003T120000-0400~fixture.log"),
        ),
        Some(fixture.root.path().join("wrong.log")),
        "20261003T120000-0400".into(),
    );
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "session::recording::tests::nested_children_keep_the_exact_owning_command",
        ])
        .env(CHILD, fixture.root.path())
        .env("THIRDSHIFT_RECORDING_DEPTH", "8")
        .env(PARENT_OWNER, "stale.log");
    recording.inherit_command(&mut child);
    let output = child.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        recording.notification_lines(),
        [
            "- implement: claude · reported-model (reported) · session effort: default effort",
            "- implement: claude · reported-model (reported) · session effort: default effort",
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
            let recording = Recording::new(
                Some(root.join("7-20261003T120000-0400.log")),
                None,
                "20261003T120000-0400".into(),
            );
            let error = recording
                .run(
                    "implement",
                    &Choice::default(),
                    Command::new("/bin/sh").args([
                        "-c",
                        &format!("echo '{MODEL}'; kill -INT \"$PPID\"; sleep 5"),
                    ]),
                    Input::default(),
                    &root.join("7-20261003T120000-0400-implement.jsonl"),
                    Harness::Claude.adapter().interpretation(root, ""),
                )
                .unwrap_err();
            assert_eq!(error.to_string(), "interrupted");
            assert_eq!(
                recording.notification_lines(),
                [
                    "- implement: claude · reported-model (reported) · session effort: default effort"
                ]
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
            Input::default(),
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
                Input::default(),
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
                Input::default(),
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
                Input::default(),
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

fn review_trace_records(
    path: &Path,
    resource: &str,
    scope: &str,
    records: &str,
) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: serde_json::Value = serde_json::from_str(line).unwrap();
            request[resource][0][scope][0][records]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

#[test]
fn review_session_totals_are_not_replaced_by_one_models_usage() {
    let fixture = Fixture::new();
    fixture.run("implement", r#"
echo '{"type":"result","subtype":"success","usage":{"input_tokens":100,"output_tokens":20},"total_cost_usd":1.0,"modelUsage":{"model-a":{"inputTokens":40,"outputTokens":8,"costUSD":0.4},"model-b":{"inputTokens":60,"outputTokens":12,"costUSD":0.6}}}'
"#, &Choice::default()).unwrap();
    let spans = review_trace_records(
        &fixture.log("implement").with_extension("spans.jsonl"),
        "resourceSpans",
        "scopeSpans",
        "spans",
    );
    let session = spans
        .iter()
        .find(|span| span["name"] == "invoke_agent implement")
        .unwrap();
    let attrs = session["attributes"].as_array().unwrap();
    let value = |key: &str| attrs.iter().find(|attr| attr["key"] == key).unwrap()["value"].clone();
    assert_eq!(
        value("gen_ai.usage.input_tokens"),
        serde_json::json!({"intValue":"100"})
    );
    assert_eq!(
        value("gen_ai.usage.output_tokens"),
        serde_json::json!({"intValue":"20"})
    );
    assert_eq!(
        value("thirdshift.cost.usd"),
        serde_json::json!({"doubleValue":1.0})
    );
}

#[test]
fn review_assistant_message_log_links_to_its_model_call() {
    let fixture = Fixture::new();
    fixture.run("implement", r#"
echo '{"type":"assistant","message":{"id":"m1","model":"test-model","content":[{"type":"text","text":"Hello."}]}}'
echo '{"type":"result","subtype":"success","result":"Hello."}'
"#, &Choice::default()).unwrap();
    let spans = review_trace_records(
        &fixture.log("implement").with_extension("spans.jsonl"),
        "resourceSpans",
        "scopeSpans",
        "spans",
    );
    let model = spans
        .iter()
        .find(|span| span["name"] == "model m1")
        .unwrap();
    let logs = review_trace_records(
        &fixture.log("implement").with_extension("logs.jsonl"),
        "resourceLogs",
        "scopeLogs",
        "logRecords",
    );
    let assistant = logs
        .iter()
        .find(|record| {
            let raw: serde_json::Value =
                serde_json::from_str(record["body"]["stringValue"].as_str().unwrap()).unwrap();
            raw["type"] == "assistant"
        })
        .unwrap();
    assert_eq!(assistant["spanId"], model["spanId"]);
}

#[test]
fn review_retained_raw_model_record_has_call_context_and_attributes() {
    let fixture = Fixture::new();
    let retained = r#"{"recorded_at":1000,"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"retained-model","usage":{"input_tokens":8,"output_tokens":4}}}}"#;
    let dir = fixture.root.path().join("sessions/s1");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("session.jsonl"), retained).unwrap();
    fixture
        .recording
        .run(
            "implement",
            &Choice::default(),
            Command::new("/bin/sh").args(["-c", &format!("echo '{INIT}'")]),
            Input::default(),
            &fixture.log("implement"),
            crate::harness::interpretation_tests::recording_interpretation(
                fixture.root.path(),
                Harness::Muse,
            ),
        )
        .unwrap();
    let spans = review_trace_records(
        &fixture.log("implement").with_extension("spans.jsonl"),
        "resourceSpans",
        "scopeSpans",
        "spans",
    );
    let model = spans
        .iter()
        .find(|span| span["name"].as_str().unwrap().starts_with("model "))
        .unwrap();
    let logs = review_trace_records(
        &fixture.log("implement").with_extension("logs.jsonl"),
        "resourceLogs",
        "scopeLogs",
        "logRecords",
    );
    let raw = logs
        .iter()
        .find(|record| record["body"]["stringValue"] == retained)
        .unwrap();
    assert_eq!(raw["spanId"], model["spanId"]);
    assert!(raw["attributes"].as_array().unwrap().iter().any(|attr| {
        attr["key"] == "gen_ai.operation.name" && attr["value"]["stringValue"] == "chat"
    }));
}

#[test]
fn review_streamed_model_records_keep_their_call_association() {
    let fixture = Fixture::new();
    fixture.run("implement", r#"
echo '{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1","model":"test-model","content":[]}}}'
echo '{"type":"assistant","message":{"id":"m1","model":"test-model","content":[{"type":"text","text":"Hello."}]}}'
echo '{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}}'
echo '{"type":"stream_event","event":{"type":"message_stop"}}'
echo '{"type":"result","subtype":"success","result":"Hello."}'
"#, &Choice::default()).unwrap();
    let spans = review_trace_records(
        &fixture.log("implement").with_extension("spans.jsonl"),
        "resourceSpans",
        "scopeSpans",
        "spans",
    );
    let model = spans
        .iter()
        .find(|span| span["name"] == "model m1")
        .unwrap();
    let logs = review_trace_records(
        &fixture.log("implement").with_extension("logs.jsonl"),
        "resourceLogs",
        "scopeLogs",
        "logRecords",
    );
    let model_records: Vec<_> = logs
        .iter()
        .filter(|record| {
            let raw: serde_json::Value =
                serde_json::from_str(record["body"]["stringValue"].as_str().unwrap()).unwrap();
            raw["type"] != "result"
        })
        .collect();
    assert_eq!(model_records.len(), 4);
    for record in model_records {
        assert_eq!(record["spanId"], model["spanId"], "{}", record["body"]);
    }
}
