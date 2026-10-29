use super::*;
use crate::harness::Harness;
use crate::test_support::native_codex_command;
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
            Command::new("/bin/sh")
                .args(["-c", script])
                .current_dir(self.root.path()),
            Input::default(),
            &self.log(name),
        )
    }
}

const MODEL: &str = r#"{"type":"assistant","message":{"model":"reported-model","content":[]}}"#;

#[test]
fn claude_stopped_background_agent_is_one_killed_work_item() {
    let fixture = Fixture::new();
    let _ = fixture.run(
        "implement",
        r#"
echo '{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"agent-tool","name":"Agent","input":{"description":"Review","run_in_background":true}}]}}'
echo '{"type":"system","subtype":"task_started","task_id":"background-task","tool_use_id":"agent-tool","task_type":"local_agent"}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent-tool","content":"Async agent launched"}]}}'
echo '{"type":"system","subtype":"task_notification","task_id":"background-task","status":"stopped","summary":"Terminated"}'
echo '{"type":"result","subtype":"success","result":"Done"}'
"#,
        &Choice::default(),
    );
    let measured = crate::logs::session_signals::read(&fixture.log("implement"))
        .unwrap()
        .unwrap();
    let measured = serde_json::to_value(measured).unwrap();
    assert_eq!(measured["signals"]["killed_work"], 1, "{measured:#}");
}

#[test]
fn session_signals_count_calls_once_after_late_permission_metadata() {
    let fixture = Fixture::new();
    fixture.run("implement", r#"
echo '{"type":"assistant","message":{"id":"m1","model":"model-a","content":[{"type":"tool_use","id":"a","name":"Bash","input":{"command":"false","timeout":1}},{"type":"tool_use","id":"b","name":"Bash","input":{"timeout":1,"command":"false"}},{"type":"tool_use","id":"c","name":"Read","input":{"command":"false","timeout":1}}]}}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","is_error":true,"content":"failed"},{"type":"tool_result","tool_use_id":"b","is_error":true,"content":"opaque failure"},{"type":"tool_result","tool_use_id":"c","is_error":false,"content":"ok"}]}}'
echo '{"type":"result","subtype":"success","permission_denials":[{"tool_use_id":"b","tool_name":"Bash"}],"usage":{"input_tokens":30,"output_tokens":8},"total_cost_usd":0.5,"modelUsage":{"model-a":{"inputTokens":10,"outputTokens":3,"cacheReadInputTokens":2,"cacheCreationInputTokens":1},"model-b":{"inputTokens":20,"outputTokens":5}}}'
"#, &Choice::default()).unwrap();
    let measured = crate::logs::session_signals::read(&fixture.log("implement"))
        .unwrap()
        .unwrap();
    let signals = serde_json::to_value(measured).unwrap()["signals"].clone();
    assert_eq!(signals["tool_errors"], 1);
    assert_eq!(signals["permission_denials"], 1);
    assert_eq!(signals["repeated_tool_calls"], 1);
    assert_eq!(signals["cost_usd"], 0.5);
    assert_eq!(
        signals["tokens_by_model"],
        serde_json::json!([
            {"model":"model-a","input_tokens":13,"output_tokens":3,"cache_read_tokens":2,"cache_write_tokens":1},
            {"model":"model-b","input_tokens":20,"output_tokens":5,"cache_read_tokens":null,"cache_write_tokens":null}
        ])
    );
}

#[test]
fn session_signals_from_each_harness_fixture_keep_reported_usage_and_cost() {
    use serde_json::json;
    for (harness, expected) in [
        (
            Harness::Claude,
            json!({"tokens_by_model":[{"model":"claude-opus-4-8",
            "input_tokens":13,"output_tokens":4,"cache_read_tokens":2,"cache_write_tokens":3}],
            "cost_usd":0.0,"killed_work":0}),
        ),
        (
            Harness::Codex,
            json!({"tokens_by_model":[{"model":null,
            "input_tokens":100,"output_tokens":7,"cache_read_tokens":20,"cache_write_tokens":null}],
            "cost_usd":null,"killed_work":2}),
        ),
        (
            Harness::Agy,
            json!({"tokens_by_model":[{"model":"gemini-test",
            "input_tokens":8,"output_tokens":4,"cache_read_tokens":null,"cache_write_tokens":null}],
            "cost_usd":null,"killed_work":0}),
        ),
        (
            Harness::Grok,
            json!({"tokens_by_model":[{"model":"grok-4.7",
            "input_tokens":25,"output_tokens":6,"cache_read_tokens":2,"cache_write_tokens":3}],
            "cost_usd":null,"killed_work":0}),
        ),
        (
            Harness::Muse,
            json!({"tokens_by_model":[],"cost_usd":null,"killed_work":0}),
        ),
        (
            Harness::OpenCode,
            json!({"tokens_by_model":[{"model":null,
            "input_tokens":7515,"output_tokens":23,"cache_read_tokens":7424,"cache_write_tokens":0}],
            "cost_usd":0.02,"killed_work":0}),
        ),
    ] {
        let fixture = Fixture::new();
        let protocol = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/normalized-{}.jsonl",
            harness.name()
        ));
        let choice = Choice {
            harness,
            ..Choice::default()
        };
        let log = fixture.log("implement");
        let _ = fixture.recording.run(
            "implement",
            &choice,
            Command::new("cat")
                .arg(protocol)
                .current_dir(fixture.root.path()),
            Input::default(),
            &log,
        );
        let measured = crate::logs::session_signals::read(&log).unwrap().unwrap();
        let measured = serde_json::to_value(measured).unwrap();
        for field in ["tokens_by_model", "cost_usd", "killed_work"] {
            assert_eq!(
                measured["signals"][field],
                expected[field],
                "{}: {field}",
                harness.name()
            );
        }
        for counter in ["tool_errors", "permission_denials", "repeated_tool_calls"] {
            assert_eq!(
                measured["signals"][counter],
                0,
                "{}: {counter}",
                harness.name()
            );
        }
    }
}

#[test]
fn explicit_launch_home_stops_private_execution_on_native_codex_quota() {
    for scoped in [false, true] {
        let fixture = Fixture::new();
        let scope = crate::scope::testing::Fixture::new();
        let mut command =
            native_codex_command(fixture.root.path(), "s1", fixture.root.path(), "current");
        command.current_dir(fixture.root.path());
        let mut invocation: Box<dyn crate::process::Invocation> = if scoped {
            Box::new(scope.launch(command))
        } else {
            Box::new(command)
        };
        let choice = Choice {
            harness: Harness::Codex,
            ..Choice::default()
        };
        let result = Recording::for_trial().run(
            "trial",
            &choice,
            invocation.as_mut(),
            Input {
                private: true,
                deadline: Some(Instant::now() + Duration::from_secs(5)),
                ..Input::default()
            },
            &fixture.log("trial"),
        );
        assert!(
            result.is_err(),
            "native warning must stop private execution (scoped: {scoped})"
        );
        assert!(!fixture.root.path().join("continued").exists());
        let trace = fs::read_to_string(fixture.log("trial").with_extension("logs.jsonl")).unwrap();
        assert!(trace.contains("used_percent"), "{trace}");
    }
}

#[test]
fn stale_or_unrelated_native_quotas_do_not_stop_private_execution() {
    for scoped in [false, true] {
        for (id, foreign_directory, timestamp) in [
            ("s1", false, "2000-01-01T00:00:00Z"),
            ("another-session", false, "current"),
            ("s1", true, "current"),
        ] {
            let fixture = Fixture::new();
            let scope = crate::scope::testing::Fixture::new();
            let directory = if foreign_directory {
                fixture.root.path().join("other")
            } else {
                fixture.root.path().to_path_buf()
            };
            let mut command = native_codex_command(fixture.root.path(), id, &directory, timestamp);
            command.current_dir(fixture.root.path());
            let mut invocation: Box<dyn crate::process::Invocation> = if scoped {
                Box::new(scope.launch(command))
            } else {
                Box::new(command)
            };
            Recording::for_trial()
                .run(
                    "trial",
                    &Choice {
                        harness: Harness::Codex,
                        ..Choice::default()
                    },
                    invocation.as_mut(),
                    Input {
                        private: true,
                        deadline: Some(Instant::now() + Duration::from_secs(5)),
                        ..Input::default()
                    },
                    &fixture.log("trial"),
                )
                .unwrap();
            assert!(fixture.root.path().join("continued").exists());
            let trace =
                fs::read_to_string(fixture.log("trial").with_extension("logs.jsonl")).unwrap();
            assert!(!trace.contains("used_percent"), "{trace}");
        }
    }
}

#[test]
fn launch_directory_is_resolved_without_canonicalizing_its_spelling() {
    let parent = std::env::current_dir().unwrap();
    let relative = Path::new("..").join(parent.file_name().unwrap()).join(".");
    for directory in [None, Some(relative), Some(parent.join("."))] {
        let fixture = Fixture::new();
        let expected = directory
            .as_ref()
            .map_or_else(|| parent.clone(), |dir| parent.join(dir));
        let mut launched = native_codex_command(fixture.root.path(), "s1", &expected, "current");
        if let Some(directory) = directory {
            launched.current_dir(directory);
        }
        let result = Recording::for_trial().run(
            "trial",
            &Choice {
                harness: Harness::Codex,
                ..Choice::default()
            },
            &mut launched,
            Input {
                private: true,
                deadline: Some(Instant::now() + Duration::from_secs(5)),
                ..Input::default()
            },
            &fixture.log("trial"),
        );
        assert!(
            result.is_err(),
            "launch directory did not match native evidence"
        );
        assert!(!fixture.root.path().join("continued").exists());
        let trace = fs::read_to_string(fixture.log("trial").with_extension("logs.jsonl")).unwrap();
        assert!(trace.contains("used_percent"), "{trace}");
    }
}

#[test]
fn ordinary_and_security_policies_use_the_chosen_harness_refusal_rules() {
    let fixture = Fixture::new();
    for (harness, refusal) in [
        (
            Harness::Claude,
            r#"{"type":"system","subtype":"model_refusal_no_fallback","api_refusal_category":"cyber"}"#,
        ),
        (
            Harness::Codex,
            r#"{"type":"turn.failed","error":{"message":"Blocked by CYBERSECURITY safeguard"}}"#,
        ),
    ] {
        for policy in [Policy::Ordinary, Policy::Security] {
            let result = fixture.recording.run(
                "arbitrary label",
                &Choice {
                    harness,
                    model: Some("selected-model".into()),
                    ..Choice::default()
                },
                Command::new("/bin/sh")
                    .args(["-c", &format!("echo '{refusal}'")])
                    .current_dir(fixture.root.path()),
                Input {
                    policy,
                    ..Input::default()
                },
                &fixture.log(harness.name()),
            );
            if matches!(policy, Policy::Security) || harness == Harness::Codex {
                assert_eq!(
                    result
                        .unwrap_err()
                        .downcast_ref::<crate::harness::interpretation::SafeguardRefusal>()
                        .is_some(),
                    matches!(policy, Policy::Security)
                );
            } else {
                result.unwrap();
            }
        }
    }
}

#[test]
fn recording_uses_the_input_session_prompt_for_skill_warnings() {
    const CHILD: &str = "THIRDSHIFT_RECORDING_PROMPT";
    if std::env::var_os(CHILD).is_some() {
        let fixture = Fixture::new();
        fixture
            .recording
            .run(
                "prompt",
                &Choice {
                    harness: Harness::OpenCode,
                    ..Choice::default()
                },
                Command::new("true").current_dir(fixture.root.path()),
                Input {
                    prompt: "/thirdshift-implement issue",
                    ..Input::default()
                },
                &fixture.log("prompt"),
            )
            .unwrap();
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::recording::tests::recording_uses_the_input_session_prompt_for_skill_warnings",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("never loaded thirdshift-implement"),
        "{output:?}"
    );
}

#[test]
fn private_recording_retains_live_scope_evidence_after_events_disappear() {
    let fixture = Fixture::new();
    let scope = crate::scope::testing::Fixture::new();
    let mut command = Command::new("/bin/sh");
    command.current_dir(fixture.root.path());
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
            Command::new("cat")
                .arg(&protocol)
                .current_dir(fixture.root.path()),
            Input {
                prompt: "session prompt",
                ..Default::default()
            },
            &log,
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

const MUSE_INIT: &str = r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.lifecycle.started","payload":{}}"#;
const MUSE_MODEL: &str = r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.model.configured","payload":{"command_id":"r","model_id":"reported-model"}}"#;
const OPENCODE_INIT: &str = r#"{"type":"step_start","sessionID":"s1","part":{"messageID":"m1"}}"#;

fn with_muse_data(test: &str, check: impl FnOnce(&Path)) {
    const ROOT: &str = "THIRDSHIFT_RECORDING_MUSE";
    if let Some(root) = std::env::var_os(ROOT) {
        check(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("session::recording::tests::{test}"),
            "--nocapture",
        ])
        .env(ROOT, root.path())
        .env("XDG_DATA_HOME", root.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

fn muse_log(root: &Path, text: &str) {
    let dir = root.join("muse/sessions/s1");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("session.jsonl"), text).unwrap();
}

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
    with_muse_data(
        "retained_models_replace_stream_evidence_including_an_empty_authoritative_set",
        |root| {
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
                muse_log(root, retained);
                let choice = Choice {
                    harness: Harness::Muse,
                    model: Some("requested-model".into()),
                    ..Choice::default()
                };
                let ended = fixture
                    .recording
                    .run(
                        "implement",
                        &choice,
                        Command::new("/bin/sh")
                            .args(["-c", &format!("echo '{MUSE_INIT}'; echo '{MUSE_MODEL}'")])
                            .current_dir(fixture.root.path()),
                        Input::default(),
                        &fixture.log("implement"),
                    )
                    .unwrap();
                assert!(ended.killed.is_empty());
                let trace =
                    fs::read_to_string(fixture.log("implement").with_extension("logs.jsonl"))
                        .unwrap();
                assert!(
                    crate::otlp_jsonl::Reader::<_, crate::otlp_jsonl::Logs>::new(trace.as_bytes())
                        .map(
                            |request| request.unwrap().resource_logs[0].scope_logs[0].log_records
                                [0]
                            .body
                            .clone()
                        )
                        .any(|body| body
                            == Some(crate::otlp_jsonl::AnyValue::String(retained.into()))),
                    "retained Harness event must survive as a raw body"
                );
                assert_eq!(
                    fixture.recording.notification_lines(),
                    [format!(
                        "- implement: muse · {expected} · session effort: default effort"
                    )]
                );
            }
        },
    );
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
fn stream_only_recording_never_launches_a_retained_export() {
    with_export(
        "stream_only_recording_never_launches_a_retained_export",
        r#"#!/bin/sh
touch "$THIRDSHIFT_RECORDING_EXPORT/exported"
echo '{"info":{"outcome":"completed"},"messages":[{"type":"assistant","model":{"id":"retained-model"},"content":[]}]}'
"#,
        |root| {
            let fixture = Fixture::new();
            fixture
                .recording
                .run(
                    "scoring-grader",
                    &Choice {
                        harness: Harness::OpenCode,
                        ..Choice::default()
                    },
                    Command::new("/bin/sh")
                        .args(["-c", &format!("echo '{OPENCODE_INIT}'")])
                        .current_dir(fixture.root.path()),
                    Input {
                        policy: Policy::StreamOnly,
                        private: true,
                        ..Input::default()
                    },
                    &fixture.log("scoring-grader"),
                )
                .unwrap();
            assert!(!root.join("exported").exists());
            let trace =
                fs::read_to_string(fixture.log("scoring-grader").with_extension("logs.jsonl"))
                    .unwrap();
            assert!(trace.contains("step_start"), "{trace}");
            assert!(!trace.contains("retained-model"), "{trace}");
        },
    );
}

#[test]
fn absent_completion_report_preserves_the_stream_checkpoint() {
    with_export(
        "absent_completion_report_preserves_the_stream_checkpoint",
        r#"#!/bin/sh
cp "$THIRDSHIFT_RECORDING_EXPORT/7-20261003T120000-0400-implement.models.json" "$THIRDSHIFT_RECORDING_EXPORT/checkpoint.json"
kill -INT "$PPID"
echo '{"info":{"outcome":"completed"},"messages":[{"type":"assistant","model":{"id":"retained-model"},"content":[]}]}'
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
                    &Choice {
                        harness: Harness::OpenCode,
                        ..Choice::default()
                    },
                    Command::new("/bin/sh")
                        .args(["-c", &format!("echo '{OPENCODE_INIT}'")])
                        .current_dir(root),
                    Input::default(),
                    &log,
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
                    "- implement: opencode · Harness default (actual model not reported) · session effort: default effort"
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
                    &Choice {
                        harness: Harness::OpenCode,
                        ..Choice::default()
                    },
                    Command::new("/bin/sh")
                        .args(["-c", &format!("echo '{OPENCODE_INIT}'")])
                        .current_dir(root),
                    Input::default(),
                    &log,
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
                    "- implement: opencode · provider/retained-model (reported) · session effort: default effort"
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
            Command::new("/bin/sh")
                .args(["-c", &script])
                .current_dir(fixture.root.path()),
            Input::default(),
            &log,
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
            Command::new("/bin/sh")
                .args(["-c", &format!("echo '{MODEL}'")])
                .current_dir(fixture.root.path()),
            Input::default(),
            &fixture.log("implement"),
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
                Command::new("/bin/sh")
                    .args(["-c", &format!("echo '{MODEL}'")])
                    .current_dir(root),
                Input::default(),
                &recording
                    .session_path(root, &depth.to_string(), "implement")
                    .unwrap(),
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
                    Command::new("/bin/sh")
                        .args([
                            "-c",
                            &format!("echo '{MODEL}'; kill -INT \"$PPID\"; sleep 5"),
                        ])
                        .current_dir(root),
                    Input::default(),
                    &root.join("7-20261003T120000-0400-implement.jsonl"),
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
            Command::new(fixture.root.path().join("missing-cli")).current_dir(fixture.root.path()),
            Input::default(),
            &fixture.log("implement"),
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
        let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
        muse_log(&data, &[
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"first"}}}"#,
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"second"}}}"#,
            r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"first"}}}"#,
        ].join("\n"));
        let choice = Choice {
            harness: Harness::Muse,
            ..Choice::default()
        };
        fixture
            .recording
            .run(
                "retained",
                &choice,
                Command::new("/bin/sh")
                    .args(["-c", &format!("echo '{MUSE_INIT}'; echo '{MUSE_MODEL}'")])
                    .current_dir(fixture.root.path()),
                Input::default(),
                &fixture.log("retained"),
            )
            .unwrap();
        muse_log(&data, "");
        fixture
            .recording
            .run(
                "empty",
                &choice,
                Command::new("/bin/sh")
                    .args(["-c", &format!("echo '{MUSE_INIT}'; echo '{MUSE_MODEL}'")])
                    .current_dir(fixture.root.path()),
                Input::default(),
                &fixture.log("empty"),
            )
            .unwrap();
        fixture
            .recording
            .run(
                "live",
                &Choice::default(),
                Command::new("/bin/sh")
                    .args(["-c", &format!("echo '{MODEL}'; echo '{MODEL}'")])
                    .current_dir(fixture.root.path()),
                Input {
                    policy: Policy::Security,
                    ..Input::default()
                },
                &fixture.log("live"),
            )
            .unwrap();
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "session::recording::tests::completion_progress_comes_from_interpretation_without_duplicate_live_lines", "--nocapture"])
        .env(CHILD, "1").env("XDG_DATA_HOME", data.path()).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let models: Vec<_> = stderr
        .lines()
        .filter_map(|line| match progress::ChildLine::of(line) {
            progress::ChildLine::Own(message) if message.contains(": Model:") => Some(message),
            progress::ChildLine::Own(_)
            | progress::ChildLine::Relayed
            | progress::ChildLine::Continuation => None,
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
    with_muse_data(
        "review_retained_raw_model_record_has_call_context_and_attributes",
        |root| {
            let fixture = Fixture::new();
            let retained = r#"{"recorded_at":1000,"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","model":"retained-model","usage":{"input_tokens":8,"output_tokens":4}}}}"#;
            muse_log(root, retained);
            fixture
                .recording
                .run(
                    "implement",
                    &Choice {
                        harness: Harness::Muse,
                        ..Choice::default()
                    },
                    Command::new("/bin/sh")
                        .args(["-c", &format!("echo '{MUSE_INIT}'")])
                        .current_dir(fixture.root.path()),
                    Input::default(),
                    &fixture.log("implement"),
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
        },
    );
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
