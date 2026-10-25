use super::*;
use crate::harness::Choice;
use crate::scope::testing::{Fixture, REMOVE_OBSERVED_EVENTS, with_grader};
use crate::test_support::write_executable;
use std::process::Command;

// Environment and Command logging are process-wide. Each exercise runs in
// its own test process; no live Harness or user manager is involved.
fn isolated(name: &str, sandboxed: bool, exercise: impl FnOnce(&Path)) {
    const ROOT: &str = "THIRDSHIFT_PRIVATE_SESSION_TEST";
    if let Some(root) = std::env::var_os(ROOT) {
        exercise(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("session::private::tests::{name}"),
            "--nocapture",
        ])
        .env(ROOT, root.path())
        .env("HOME", root.path().join("home"))
        .env("PATH", path)
        .env(
            "THIRDSHIFT_TRIAL_NO_SANDBOX",
            if sandboxed { "" } else { "1" },
        )
        .env("THIRDSHIFT_LOG_OWNER", "stale-command-owner")
        .env(
            logs::trace::TRIAL_CONTEXT,
            serde_json::to_string(&logs::trace::Context::of_session(
                &root.path().join("trial"),
            ))
            .unwrap(),
        );
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("private completion"),
        "{output:?}"
    );
}

fn auxiliary(
    fixture: &std::rc::Rc<Fixture>,
    choice: &Choice,
    script: &str,
    environment: &[(&str, &Path)],
) -> Result<()> {
    let binary = fixture.path("pinned-harness");
    write_executable(
        &binary,
        &format!(
            "#!/bin/sh\nif [ \"${{1-}}\" = --version ]; then echo fixture; exit 0; fi\n{script}\n"
        ),
    );
    with_grader(fixture, || {
        run(Request {
            kind: "model Grader",
            choice,
            binary: &binary,
            workspace: fixture.directory.path(),
            artifacts: fixture.directory.path(),
            prompt: "grade packet",
            timeout: Duration::from_secs(5),
            caps: &Caps::default(),
            environment,
        })
    })
}

#[test]
fn auxiliary_reads_native_codex_quota_from_its_launch_home() {
    isolated(
        "auxiliary_reads_native_codex_quota_from_its_launch_home",
        false,
        |_| {
            let fixture = Fixture::new();
            let command = crate::test_support::native_codex_command(
                fixture.directory.path(),
                "s1",
                fixture.directory.path(),
                "current",
            );
            let script = command.get_args().last().unwrap().to_str().unwrap();
            let environment: Vec<_> = command
                .get_envs()
                .map(|(name, value)| (name.to_str().unwrap(), Path::new(value.unwrap())))
                .collect();
            let error = auxiliary(
                &fixture,
                &Choice {
                    harness: crate::harness::Harness::Codex,
                    ..Choice::default()
                },
                script,
                &environment,
            )
            .unwrap_err();
            assert!(error.is::<crate::session::limits::Reached>(), "{error:#}");
            assert!(!fixture.path("continued").exists());
            let trace = fs::read_to_string(fixture.path("stdout.logs.jsonl")).unwrap();
            assert!(trace.contains("used_percent"), "{trace}");
        },
    );
}

#[test]
fn auxiliary_retains_live_kill_and_its_private_diagnostic() {
    isolated(
        "auxiliary_retains_live_kill_and_its_private_diagnostic",
        false,
        |_| {
            let fixture = Fixture::new();
            let result = auxiliary(
                &fixture,
                &Choice::default(),
                &format!(
                    "echo '{{\"type\":\"assistant\",\"message\":{{\"model\":\"fixture-model\",\"content\":[]}}}}'; \
             echo 'oom_group_kill 1' > \"$EVENTS\"; {REMOVE_OBSERVED_EVENTS}"
                ),
                &[],
            );
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("model Grader hit its memory cap"),
                "{message}"
            );
            assert!(message.contains(crate::scope::OOM_KILL_CAUSE), "{message}");
            assert!(!fixture.path("events").exists());
            assert_eq!(fixture.journal_reads(), 0);
            for extension in ["logs.jsonl", "spans.jsonl"] {
                assert!(
                    !fs::read(fixture.path("stdout.jsonl").with_extension(extension))
                        .unwrap()
                        .is_empty()
                );
            }
        },
    );
}

#[test]
fn auxiliary_uses_only_kernel_evidence_and_keeps_journal_lazy() {
    isolated(
        "auxiliary_uses_only_kernel_evidence_and_keeps_journal_lazy",
        false,
        |_| {
            for (events, journal, success, expected) in [
                (
                    Some("oom_kill 0\noom_group_kill 0"),
                    false,
                    false,
                    "session failed",
                ),
                (None, false, false, "session failed"),
                (Some("oom_kill unreadable"), false, false, "session failed"),
                (None, true, false, "hit its memory cap"),
                (Some("oom_kill 1"), true, true, ""),
            ] {
                let fixture = Fixture::new();
                fixture.journal_kill.set(journal);
                if let Some(events) = events {
                    fs::write(fixture.path("events"), events).unwrap();
                }
                let result = auxiliary(
                    &fixture,
                    &Choice::default(),
                    if success {
                        r#"echo '{"type":"result","subtype":"success","result":"graded"}'"#
                    } else {
                        "echo 'agent claims out of memory'; exit 137"
                    },
                    &[],
                );
                if success {
                    result.unwrap();
                    assert_eq!(fixture.journal_reads(), 0);
                } else {
                    let message = result.unwrap_err().to_string();
                    assert!(message.contains(expected), "{message}");
                    assert!(!message.contains("agent claims"));
                    assert!(fixture.journal_reads() > 0);
                }
            }
        },
    );
}

#[test]
fn scoring_uses_an_ordinary_unscoped_session_and_registers_its_command_models() {
    isolated(
        "scoring_uses_an_ordinary_unscoped_session_and_registers_its_command_models",
        false,
        |root| {
            let repo = crate::issue::Repo {
                owner: "acme".into(),
                name: "widgets".into(),
            };
            let packet = root.join("packet");
            fs::create_dir(&packet).unwrap();
            crate::logs::begin(crate::logs::Begin::ScoringRun);
            crate::logs::configured(&crate::config::UserConfig::load().unwrap());
            let choice = Choice {
                model: Some("requested-model".into()),
                effort: Some("high".into()),
                ..Choice::default()
            };
            crate::logs::started(crate::logs::Work::ScoringRun(&repo), &choice);
            write_executable(&root.join("bin/systemd-run"), "#!/bin/sh\nexit 99\n");
            write_executable(
                &root.join("bin/claude"),
                r#"#!/bin/sh
if [ "${1-}" = --version ]; then echo fixture; exit 0; fi
printf '%s\n' "$@" > arguments
for output in ../grader-*; do test -d "$output" || exit 98; done
sleep 0.1
echo '{"type":"assistant","message":{"model":"observed-model","content":[]}}'
echo '{"type":"result","subtype":"success","result":"private completion"}'
"#,
            );
            run_scoring(&choice, &packet, "grade packet", &repo, 7).unwrap();
            let args = fs::read_to_string(packet.join("arguments")).unwrap();
            assert!(args.contains("requested-model"), "{args}");
            assert!(!args.contains("disallowedTools"), "{args}");
            let recording = Recording::capture();
            let log = recording
                .session_path(
                    &crate::logs::session_dir(&repo),
                    "scoring",
                    "scoring-grader-7",
                )
                .unwrap();
            assert!(log.is_file(), "{}", log.display());
            let models: serde_json::Value = serde_json::from_slice(
                &fs::read(logs::session::original(&log).with_extension("models.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(models["requested"], "requested-model");
            assert_eq!(models["observed"], serde_json::json!(["observed-model"]));
            assert_eq!(
                models["command_log"],
                crate::logs::command_log_path().unwrap().to_str().unwrap()
            );
            let record: serde_json::Value = serde_json::from_slice(
                &fs::read(
                    crate::logs::command_log_path()
                        .unwrap()
                        .with_extension("command.json"),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(record["session_log"], log.to_str().unwrap());
            assert!(!fs::read_dir(root).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("grader-")
            }));
        },
    );
}

fn scoring(root: &Path, choice: &Choice) -> Result<()> {
    let repo = Repo {
        owner: "acme".into(),
        name: "widgets".into(),
    };
    logs::begin(logs::Begin::ScoringRun);
    logs::configured(&crate::config::UserConfig::load().unwrap());
    logs::started(logs::Work::ScoringRun(&repo), choice);
    run_scoring(choice, root, "grade packet", &repo, 1)
}

#[test]
fn scoring_stream_only_never_launches_a_retained_export() {
    isolated(
        "scoring_stream_only_never_launches_a_retained_export",
        false,
        |root| {
            write_executable(
                &root.join("bin/opencode"),
                r#"#!/bin/sh
if [ "$1" = export ]; then touch exported; exit 99; fi
echo '{"type":"step_start","sessionID":"s1","part":{"messageID":"m1"}}'
"#,
            );
            scoring(
                root,
                &Choice {
                    harness: crate::harness::Harness::OpenCode,
                    ..Choice::default()
                },
            )
            .unwrap();
            assert!(!root.join("exported").exists());
        },
    );
}

#[test]
fn scoring_maps_sandbox_startup_failure_after_a_successful_probe() {
    isolated(
        "scoring_maps_sandbox_startup_failure_after_a_successful_probe",
        true,
        |root| {
            write_executable(&root.join("bin/claude"), "#!/bin/sh\nexit 99\n");
            write_executable(&root.join("bin/socat"), "#!/bin/sh\nexit 99\n");
            write_executable(
                &root.join("bin/bwrap"),
                r#"#!/bin/sh
for last; do :; done
if [ "$last" = /bin/true ]; then exit 0; fi
echo 'bwrap: fixture startup failure' >&2
exit 1
"#,
            );
            let error = scoring(root, &Choice::default()).unwrap_err();
            assert!(error.is::<sandbox::Unavailable>(), "{error:#}");
            assert!(
                format!("{error:#}").contains("fixture startup failure"),
                "{error:#}"
            );
        },
    );
}

#[test]
fn auxiliary_keeps_its_pinned_trial_invocation_caps_deadline_and_artifacts() {
    isolated(
        "auxiliary_keeps_its_pinned_trial_invocation_caps_deadline_and_artifacts",
        false,
        |root| {
            write_executable(&root.join("bin/claude"), "#!/bin/sh\nexit 99\n");
            write_executable(&root.join("bin/systemctl"), "#!/bin/sh\necho running\n");
            write_executable(&root.join("bin/journalctl"), "#!/bin/sh\nexit 0\n");
            write_executable(
                &root.join("bin/systemd-run"),
                r#"#!/bin/sh
if [ "${1-}" = --version ]; then echo fixture; exit 0; fi
printf '%s\n' "$@" > scope-arguments
while [ "$1" != -- ]; do shift; done
shift
# The test binary has no --oom-exec command; execute the pinned Harness
# after recording the production scope command, including that wrapper.
shift 2
exec "$@"
"#,
            );
            let binary = root.join("pinned-harness");
            write_executable(
                &binary,
                r#"#!/bin/sh
printf '%s\n' "$@" > harness-arguments
echo 'private timeout evidence' >&2
echo '{"type":"assistant","message":{"model":"fixture-model","content":[]}}'
sleep 3
touch continued
"#,
            );
            let error = run(Request {
                kind: "model Grader",
                choice: &Choice::default(),
                binary: &binary,
                workspace: root,
                artifacts: root,
                prompt: "grade packet",
                timeout: Duration::from_millis(200),
                caps: &Caps {
                    high: "37M".into(),
                    max: "41M".into(),
                    swap_max: "0".into(),
                },
                environment: &[],
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("model Grader session failed"),
                "{error:#}"
            );
            assert!(!root.join("continued").exists());
            let args = fs::read_to_string(root.join("harness-arguments")).unwrap();
            assert!(
                args.contains("--disallowedTools=WebSearch,WebFetch"),
                "{args}"
            );
            let scope = fs::read_to_string(root.join("scope-arguments")).unwrap();
            for expected in [
                "--scope",
                "MemoryHigh=37M",
                "MemoryMax=41M",
                "MemorySwapMax=0",
                "--oom-exec",
                binary.to_str().unwrap(),
            ] {
                assert!(scope.contains(expected), "{scope}");
            }
            assert!(
                fs::read_to_string(root.join("stderr"))
                    .unwrap()
                    .contains("private timeout evidence")
            );
            for file in ["scope", "stdout.logs.jsonl", "stdout.spans.jsonl"] {
                assert!(!fs::read(root.join(file)).unwrap().is_empty(), "{file}");
            }
        },
    );
}

#[test]
fn auxiliary_keeps_trial_ancestry_and_clears_stale_command_inheritance() {
    isolated(
        "auxiliary_keeps_trial_ancestry_and_clears_stale_command_inheritance",
        false,
        |root| {
            let fixture = Fixture::new();
            auxiliary(
                &fixture,
                &Choice::default(),
                r#"
test -z "${THIRDSHIFT_LOG_OWNER+x}" || exit 91
test -z "${THIRDSHIFT_TRIAL_TRACE_CONTEXT+x}" || exit 92
echo '{"type":"result","subtype":"success","result":"private completion"}'
"#,
                &[],
            )
            .unwrap();
            let request: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(fixture.path("stdout.spans.jsonl"))
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap(),
            )
            .unwrap();
            let context = logs::trace::Context::of_session(&root.join("trial"));
            let session = request["resourceSpans"][0]["scopeSpans"][0]["spans"]
                .as_array()
                .unwrap()
                .iter()
                .find(|span| span["name"] == "invoke_agent model Grader")
                .unwrap();
            assert_eq!(
                session["traceId"],
                serde_json::to_value(context.trace).unwrap()
            );
            assert_eq!(
                session["parentSpanId"],
                serde_json::to_value(context.stage).unwrap()
            );
            let models: serde_json::Value =
                serde_json::from_slice(&fs::read(fixture.path("stdout.models.json")).unwrap())
                    .unwrap();
            assert!(models["command_log"].is_null());
        },
    );
}
