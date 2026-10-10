use super::*;
use std::os::unix::process::ExitStatusExt;

#[test]
fn agy_security_progress_names_the_resolved_model() {
    let mut interpretation = stream(Harness::Agy, "").for_security(Some("gemini-3.8-flash"));
    let lines = interpretation.condense(
        r#"{"event":"init","conversation_id":"s1","init":{"model":"gemini-3.8-flash-high"}}"#,
    );
    assert!(
        lines.contains(&"Model: gemini-3.8-flash-high".to_string()),
        "{lines:?}"
    );
}

#[test]
fn a_security_session_can_discuss_the_cyber_marker_without_being_refused() {
    let mut interpretation = stream(Harness::Claude, "").for_security(None);
    interpretation.condense(
        r#"{"type":"result","subtype":"success","result":"The [cyber] refusal test passed.\nSecurity audit: complete"}"#,
    );
    let ended = finish(interpretation).outcome.unwrap();
    assert_eq!(
        ended.final_message.as_deref(),
        Some("The [cyber] refusal test passed.\nSecurity audit: complete")
    );
}

#[test]
fn a_terminal_claude_cyber_refusal_overrides_a_successful_security_result() {
    let event = r#"{"type":"system","subtype":"model_refusal_no_fallback","api_refusal_category":"cyber","content":"Private safeguard explanation."}"#;
    for security in [false, true] {
        let mut interpretation = stream(Harness::Claude, "");
        if security {
            interpretation = interpretation.for_security(None);
        }
        interpretation.condense(event);
        interpretation.condense(
            r#"{"type":"result","subtype":"success","result":"Security audit: complete"}"#,
        );
        let completion = finish(interpretation);
        if security {
            let error = completion.outcome.unwrap_err();
            assert_eq!(
                error.downcast_ref::<interpretation::SafeguardRefusal>(),
                Some(&interpretation::SafeguardRefusal::ClaudeCyber)
            );
        } else {
            assert!(completion.outcome.is_ok());
        }
    }
}

#[test]
fn security_model_progress_names_only_new_main_loop_answers() {
    let mut interpretation = stream(Harness::Claude, "").for_security(Some("opus"));
    for ignored in [
        r#"{"type":"system","subtype":"init","model":"requested-model"}"#,
        r#"{"type":"assistant","parent_tool_use_id":"child","message":{"model":"child-model","content":[]}}"#,
        r#"{"type":"result","subtype":"success","modelUsage":{"child-model":{"inputTokens":100}}}"#,
    ] {
        assert!(
            interpretation
                .condense(ignored)
                .iter()
                .all(|line| !line.starts_with("Model:"))
        );
    }
    let answer = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[]}}"#;
    assert_eq!(
        interpretation.condense(answer),
        vec!["Model: claude-opus-4-8"]
    );
    assert!(interpretation.condense(answer).is_empty());
}

#[test]
fn claude_completion_keeps_the_last_result_and_the_last_complete_usage() {
    let mut interpretation = Harness::Claude
        .adapter()
        .interpretation(Path::new("/work"), "");
    for line in [
        r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"none"}"#,
        r#"{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":34,"total_cost_usd":1.8249}"#,
        r#"{"type":"result","subtype":"success","result":"Done."}"#,
    ] {
        interpretation.condense(line);
    }
    let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(0)));
    let ended = completion.outcome.unwrap();
    assert_eq!(ended.session_id.as_deref(), Some("s1"));
    assert_eq!(ended.final_message.as_deref(), Some("Done."));
    assert_eq!(
        completion.report.unwrap().summary.as_deref(),
        Some("34 turns, $1.82 at API prices")
    );
}

pub(crate) fn stream(harness: Harness, prompt: &str) -> interpretation::Interpretation {
    // A missing working directory makes optional OpenCode acquisition fail
    // locally, so in-process fallback tests never invoke a machine's CLI.
    let absent = tempfile::tempdir().unwrap().path().join("absent");
    let worktree = if harness == Harness::OpenCode {
        absent.as_path()
    } else {
        Path::new("/work/widgets-issue-7")
    };
    harness.adapter().interpretation(worktree, prompt)
}

pub(crate) fn finish(stream: interpretation::Interpretation) -> interpretation::Completion {
    stream.finish(Ok(std::process::ExitStatus::from_raw(0)))
}

pub(crate) fn lines(
    harness: Harness,
    events: &[serde_json::Value],
) -> (interpretation::Completion, Vec<Vec<String>>) {
    let mut interpretation = stream(harness, "");
    let lines = events
        .iter()
        .map(|event| interpretation.condense(&event.to_string()))
        .collect();
    (finish(interpretation), lines)
}

#[test]
fn every_harness_accepts_missing_final_data_without_inventing_usage_or_a_session_id() {
    for harness in Harness::ALL {
        let completion = finish(stream(harness, ""));
        let ended = completion.outcome.unwrap();
        assert_eq!(ended.session_id, None, "{harness:?}");
        assert_eq!(ended.final_message, None, "{harness:?}");
        assert!(ended.killed.is_empty(), "{harness:?}");
        assert_eq!(completion.report.unwrap().summary, None, "{harness:?}");
    }
}

#[test]
fn claude_error_subtypes_and_error_flags_fail_without_diagnostic_text() {
    for line in [
        r#"{"type":"result","subtype":"error_max_turns"}"#,
        r#"{"type":"result","subtype":"success","is_error":true}"#,
    ] {
        let mut interpretation = stream(Harness::Claude, "");
        interpretation.condense(line);
        let completion = finish(interpretation);
        assert_eq!(
            completion.outcome.unwrap_err().to_string(),
            "claude's turn failed"
        );
        assert!(completion.report.is_some());
    }
}

// Muse's data root is process configuration. Isolate it in a subprocess so
// filesystem tests use the adapter without mutating the parallel test runner.
fn with_muse_data(test: &str, run: impl FnOnce(&Path)) {
    if let Some(root) = std::env::var_os("THIRDSHIFT_TEST_MUSE_DATA") {
        run(Path::new(&root));
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("harness::interpretation_tests::{test}")])
        .env("THIRDSHIFT_TEST_MUSE_DATA", root.path())
        .env("XDG_DATA_HOME", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn muse_log(root: &Path, text: &str) {
    let dir = root.join("muse/sessions/2026/10/06/s1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("session.jsonl"), text).unwrap();
}

fn muse_reply() -> interpretation::Interpretation {
    let mut interpretation = stream(Harness::Muse, "");
    for line in [
        r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.output.delta","payload":{"text":"stream reply"}}"#,
        r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.terminal.completed","payload":{}}"#,
    ] {
        interpretation.condense(line);
    }
    interpretation
}

#[test]
fn muse_completion_recovers_the_last_reply_and_parent_usage_including_retained_transactions() {
    with_muse_data(
        "muse_completion_recovers_the_last_reply_and_parent_usage_including_retained_transactions",
        |root| {
            use serde_json::json;
            let record =
                |event| json!({"stream":{"id":"s1"},"payload":{"kind":"run","event":event}});
            let message =
                record(json!({"kind":"assistant_message_committed","text":"retained reply"}));
            let lines = [
            record(json!({"kind":"assistant_message_committed","text":"earlier reply"})),
            record(json!({"kind":"model_completed","usage":{"input_tokens":26000,"cached_tokens":12000,"output_tokens":40}})),
            json!({"stream":{"id":"child"},"payload":{"kind":"run","event":{"kind":"model_completed","usage":{"input_tokens":999999}}}}),
            record(json!({"kind":"model_completed","usage":{"input_tokens":24000,"cached_tokens":20000,"output_tokens":30}})),
            json!({"children":[{"record_json":message.to_string()}]}),
        ].map(|line| line.to_string()).join("\n");
            muse_log(root, &lines);
            let completion = finish(muse_reply());
            let ended = completion.outcome.unwrap();
            assert_eq!(ended.final_message.as_deref(), Some("retained reply"));
            assert_eq!(ended.session_id.as_deref(), Some("s1"));
            assert_eq!(
                completion.report.unwrap().summary.as_deref(),
                Some("50000 input tokens (32000 cached), 70 output tokens")
            );
        },
    );
}

#[test]
fn muse_retained_records_without_text_preserve_the_stream_reply() {
    with_muse_data(
        "muse_retained_records_without_text_preserve_the_stream_reply",
        |root| {
            muse_log(
                root,
                r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","usage":{"input_tokens":42,"output_tokens":7}}}}"#,
            );
            let completion = finish(muse_reply());
            assert_eq!(
                completion.outcome.unwrap().final_message.as_deref(),
                Some("stream reply")
            );
            assert_eq!(
                completion.report.unwrap().summary.as_deref(),
                Some("42 input tokens (0 cached), 7 output tokens")
            );
        },
    );
}

#[test]
fn unreadable_muse_records_fall_back_without_usage() {
    with_muse_data("unreadable_muse_records_fall_back_without_usage", |root| {
        for text in [
            None,
            Some("not JSON"),
            Some("{"),
            Some(r#"{"children":[{"record_json":"bad"}]}"#),
        ] {
            if let Some(text) = text {
                muse_log(root, text);
            }
            let completion = finish(muse_reply());
            assert_eq!(
                completion.outcome.unwrap().final_message.as_deref(),
                Some("stream reply")
            );
            assert_eq!(completion.report.unwrap().summary, None);
        }
    });
}

#[test]
fn a_transport_failure_preserves_recovered_usage_and_warnings_ahead_of_turn_failure() {
    with_muse_data(
        "a_transport_failure_preserves_recovered_usage_and_warnings_ahead_of_turn_failure",
        |root| {
            muse_log(
                root,
                r#"{"stream":{"id":"s1"},"payload":{"kind":"run","event":{"kind":"model_completed","usage":{"input_tokens":42,"output_tokens":7}}}}"#,
            );
            let mut interpretation = Harness::Muse
                .adapter()
                .interpretation(Path::new("/work"), "/thirdshift-implement issue-url");
            interpretation.condense(r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.terminal.failed","payload":{"reason":"quota exceeded"}}"#);
            let completion = interpretation.finish(Err(anyhow::anyhow!(
                "could not write the session log: Broken pipe"
            )));
            assert_eq!(
                completion.outcome.unwrap_err().to_string(),
                "could not write the session log: Broken pipe"
            );
            let report = completion.report.unwrap();
            assert_eq!(
                report.warnings,
                ["warning: the session never loaded thirdshift-implement with its skill tool"]
            );
            assert_eq!(
                report.summary.as_deref(),
                Some("42 input tokens (0 cached), 7 output tokens")
            );
        },
    );
}

#[test]
fn an_interrupted_opencode_check_export_returns_interrupted_without_refusal_context() {
    if std::env::var_os("THIRDSHIFT_TEST_CHECK_EXPORT_INTERRUPT").is_some() {
        crate::interrupt::install().unwrap();
        let error = opencode::check_model_and_effort(&ModelAndEffort::default()).unwrap_err();
        assert_eq!(format!("{error:#}"), "interrupted");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("opencode");
    // A local CLI records a session, then interrupts its exact owner during
    // export. The process owner must stop and join it before the check returns.
    crate::test_support::write_executable(
        &cli,
        r#"#!/bin/bash
if test "$1" = run; then
    cat >/dev/null
    echo '{"type":"text","sessionID":"s1","part":{"text":"OK"}}'
else
    kill -INT "$PPID"
    sleep 30
fi
"#,
    );
    let path = std::env::join_paths(
        std::iter::once(dir.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "harness::interpretation_tests::an_interrupted_opencode_check_export_returns_interrupted_without_refusal_context"])
        .env("THIRDSHIFT_TEST_CHECK_EXPORT_INTERRUPT", "1")
        .env("PATH", path)
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Local recording fixtures combine the real Claude stream protocol with the
/// real retained policies, so conflicting/empty evidence crosses the runner.
pub(crate) fn recording_interpretation(
    root: &Path,
    retained_harness: Harness,
) -> interpretation::Interpretation {
    let retained = match retained_harness {
        Harness::Muse => interpretation::Retained::Muse(Some(root.to_path_buf())),
        Harness::OpenCode => interpretation::Retained::OpenCode(root.to_path_buf()),
        _ => panic!("recording fixtures support only retained Harnesses"),
    };
    interpretation::Interpretation::new(
        "claude",
        Box::new(interpretation::Stream::claude()),
        retained,
    )
}

#[test]
fn assistant_models_keep_attribution_order_without_changing_root_completion() {
    for harness in [Harness::Claude, Harness::Grok] {
        for security in [false, true] {
            let mut interpretation = stream(harness, "");
            if security {
                interpretation = interpretation.for_security(Some("requested-model"));
            }
            let fixtures = [
                (
                    r#"{"type":"system","subtype":"init","session_id":"s1","model":"requested-model"}"#,
                    vec!["session started"],
                ),
                (
                    r#"{"type":"assistant","parent_tool_use_id":"child","message":{"model":"child-model","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","parent_tool_use_id":false,"message":{"model":"other-child","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","is_api_error_message":true,"message":{"model":"api-error-model","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","message":{"model":"<synthetic>","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","message":{"model":"","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","parent_tool_use_id":null,"message":{"model":"root-model","content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo check"}}]}}"#,
                    if security {
                        vec!["$ cargo check", "Model: root-model"]
                    } else {
                        vec!["$ cargo check"]
                    },
                ),
                (
                    r#"{"type":"assistant","message":{"model":"root-model","content":[]}}"#,
                    vec![],
                ),
                (
                    r#"{"type":"assistant","message":{"model":"next-model","content":[]}}"#,
                    if security {
                        vec!["Model: next-model"]
                    } else {
                        vec![]
                    },
                ),
                (
                    r#"{"type":"assistant","message":{"model":"root-model","content":[]}}"#,
                    if security {
                        vec!["Model: root-model"]
                    } else {
                        vec![]
                    },
                ),
                (
                    r#"{"type":"result","subtype":"success","result":"root reply","num_turns":2,"total_cost_usd":0.25}"#,
                    vec![],
                ),
                (
                    r#"{"type":"result","parent_tool_use_id":"child","subtype":"success","result":"child reply","modelUsage":{"usage-model":{},"child-model":{},"<synthetic>":{},"":{}}}"#,
                    vec![],
                ),
                ("not JSON", vec![]),
                ("{}", vec![]),
            ];
            for (event, expected) in fixtures {
                assert_eq!(
                    interpretation.condense(event),
                    expected,
                    "{harness:?}, Security={security}: {event}"
                );
            }
            let mut expected = vec!["child-model", "other-child", "root-model", "next-model"];
            if harness == Harness::Claude {
                expected.push("usage-model");
            }
            assert_eq!(interpretation.observed_models(), expected);
            let completion = finish(interpretation);
            assert_eq!(completion.report.unwrap().models, expected);
            let ended = completion.outcome.unwrap();
            assert_eq!(ended.session_id.as_deref(), Some("s1"));
            assert_eq!(
                ended.final_message.as_deref(),
                Some(if harness == Harness::Claude {
                    "root reply"
                } else {
                    "child reply"
                })
            );
        }
    }
}

#[test]
fn repeated_agy_initialization_keeps_model_evidence_after_identity_acceptance() {
    for security in [false, true] {
        let mut interpretation = stream(Harness::Agy, "");
        if security {
            interpretation = interpretation.for_security(Some("requested-model"));
        }
        let fixtures = [
            (
                r#"{"event":"init","init":{"model":"first-model"}}"#,
                true,
                Some("Model: first-model"),
            ),
            (
                r#"{"event":"init","conversation_id":"s1","init":{"model":"second-model"}}"#,
                true,
                Some("Model: second-model"),
            ),
            (
                r#"{"event":"init","conversation_id":"s2","parent_tool_use_id":"child","init":{"model":"child-model"}}"#,
                false,
                None,
            ),
            (
                r#"{"event":"init","init":{"model":"second-model"}}"#,
                false,
                None,
            ),
            (
                r#"{"event":"init","init":{"model":"<synthetic>"}}"#,
                false,
                None,
            ),
            (r#"{"event":"init","init":{"model":""}}"#, false, None),
            (
                r#"{"event":"init","init":{"model":"last-model"}}"#,
                false,
                Some("Model: last-model"),
            ),
            ("not JSON", false, None),
        ];
        for (event, started, model) in fixtures {
            let mut expected = Vec::new();
            if started {
                expected.push("session started");
            }
            if security && let Some(model) = model {
                expected.push(model);
            }
            assert_eq!(interpretation.condense(event), expected);
        }
        let expected = ["first-model", "second-model", "child-model", "last-model"];
        assert_eq!(interpretation.observed_models(), expected);
        let completion = finish(interpretation);
        assert_eq!(
            completion.outcome.unwrap().session_id.as_deref(),
            Some("s1")
        );
        assert_eq!(completion.report.unwrap().models, expected);
    }
}

#[test]
fn codex_starts_report_requested_or_default_progress_without_observed_models() {
    for requested in [None, Some("gpt-requested")] {
        let mut interpretation = stream(Harness::Codex, "");
        // Security can start after the decoder has accepted its identity.
        assert_eq!(
            interpretation.condense(r#"{"type":"thread.started","thread_id":"s1"}"#),
            ["session started"]
        );
        interpretation = interpretation.for_security(requested);
        let expected = if requested.is_some() {
            "Model: gpt-requested (requested)"
        } else {
            "Model: Harness default (no Model requested)"
        };
        let repeated = r#"{"type":"thread.started","thread_id":"s2","model":"not-evidence"}"#;
        assert_eq!(interpretation.condense(repeated), [expected]);
        assert!(interpretation.condense(repeated).is_empty());
        assert!(interpretation.observed_models().is_empty());
        interpretation = interpretation.for_security(requested);
        assert_eq!(interpretation.condense(repeated), [expected]);
        let completion = finish(interpretation);
        assert_eq!(
            completion.outcome.unwrap().session_id.as_deref(),
            Some("s1")
        );
        assert!(completion.report.unwrap().models.is_empty());
    }
}

#[test]
fn enabling_or_resetting_security_does_not_replay_refusals_or_clear_models() {
    let refusal =
        r#"{"type":"system","subtype":"model_refusal_no_fallback","api_refusal_category":"cyber"}"#;
    let answer = r#"{"type":"assistant","message":{"model":"root-model","content":[]}}"#;
    for reset in [false, true] {
        let mut interpretation = stream(Harness::Claude, "");
        if reset {
            interpretation = interpretation.for_security(None);
        }
        interpretation.condense(answer);
        interpretation.condense(refusal);
        interpretation = interpretation.for_security(None);
        assert_eq!(interpretation.observed_models(), ["root-model"]);
        assert_eq!(interpretation.condense(answer), ["Model: root-model"]);
        assert_eq!(interpretation.observed_models(), ["root-model"]);
        interpretation.condense(r#"{"type":"result","subtype":"success","result":"done"}"#);
        let completion = finish(interpretation);
        assert!(completion.outcome.is_ok());
        assert_eq!(completion.report.unwrap().models, ["root-model"]);
    }
}

#[test]
fn claude_refusal_predicates_survive_delegated_results_only_in_security() {
    for event in [
        r#"{"type":"system","subtype":"model_refusal_no_fallback","api_refusal_category":"cyber","parent_tool_use_id":"child"}"#,
        r#"{"type":"result","subtype":"success","parent_tool_use_id":"child","result":"  [cyber] private explanation","modelUsage":{"child-model":{}}}"#,
        r#"{"type":"result","subtype":"success","parent_tool_use_id":"child","result":"\nAPI Error: [cyber] private explanation","modelUsage":{"child-model":{}}}"#,
    ] {
        for harness in [Harness::Claude, Harness::Grok] {
            for security in [false, true] {
                let mut interpretation = stream(harness, "");
                if security {
                    interpretation = interpretation.for_security(None);
                }
                interpretation.condense(event);
                interpretation
                    .condense(r#"{"type":"result","subtype":"success","result":"root reply"}"#);
                let completion = finish(interpretation);
                if harness == Harness::Claude && security {
                    assert_eq!(
                        completion
                            .outcome
                            .unwrap_err()
                            .downcast_ref::<interpretation::SafeguardRefusal>(),
                        Some(&interpretation::SafeguardRefusal::ClaudeCyber)
                    );
                } else {
                    assert_eq!(
                        completion.outcome.unwrap().final_message.as_deref(),
                        Some("root reply")
                    );
                }
            }
        }
    }
}

#[test]
fn codex_classifies_only_terminal_cybersecurity_errors_and_keeps_refusals_sticky() {
    for terminal in [false, true] {
        for security in [false, true] {
            let mut interpretation = stream(Harness::Codex, "");
            if security {
                interpretation = interpretation.for_security(None);
            }
            let event = if terminal {
                r#"{"type":"turn.failed","error":{"message":"Blocked by CYBERSECURITY safeguard"}}"#
            } else {
                r#"{"type":"error","message":"Discussion of cybersecurity"}"#
            };
            interpretation.condense(event);
            interpretation.condense(
                r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2}}"#,
            );
            let completion = finish(interpretation);
            let report = completion.report.unwrap();
            assert_eq!(
                report.summary.as_deref(),
                Some("10 input tokens (0 cached), 2 output tokens")
            );
            assert!(report.models.is_empty());
            if terminal {
                let error = completion.outcome.unwrap_err();
                assert_eq!(
                    error.downcast_ref::<interpretation::SafeguardRefusal>(),
                    if security {
                        Some(&interpretation::SafeguardRefusal::CodexCyber)
                    } else {
                        None
                    }
                );
            } else {
                assert!(completion.outcome.is_ok());
            }
        }
    }
}

#[test]
fn retained_harnesses_do_not_invent_live_model_or_safeguard_evidence() {
    for harness in [Harness::Muse, Harness::OpenCode] {
        let mut interpretation = stream(harness, "").for_security(Some("requested-model"));
        for event in [
            r#"{"type":"assistant","message":{"model":"not-evidence"}}"#,
            r#"{"event":"init","init":{"model":"not-evidence"}}"#,
            r#"{"type":"thread.started","model":"not-evidence"}"#,
            r#"{"type":"system","subtype":"model_refusal_no_fallback","api_refusal_category":"cyber"}"#,
            r#"{"type":"result","result":"[cyber] not this dialect"}"#,
            "not JSON",
        ] {
            assert!(interpretation.condense(event).is_empty());
        }
        assert!(interpretation.observed_models().is_empty());
        let completion = finish(interpretation);
        assert!(completion.outcome.is_ok());
        assert!(completion.report.unwrap().models.is_empty());
    }
}
