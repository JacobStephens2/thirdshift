use super::*;
use std::os::unix::process::ExitStatusExt;

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
