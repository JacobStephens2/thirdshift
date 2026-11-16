use std::fs;
use std::path::Path;

use serde_json::json;

use super::{Recorder, arms, collect, web_search};
use crate::harness::Harness;
use crate::proxy::{Accounting, HarnessInvocation};
use crate::trial::{Outcome, Output, VoidCause};

fn attempt() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for directory in ["session", "output", "repository"] {
        fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    fs::write(
        root.path().join("trace-context.json"),
        serde_json::to_vec(&crate::logs::trace::Context::of_session(root.path())).unwrap(),
    )
    .unwrap();
    root
}

fn runner(root: &Path, manifest: serde_json::Value) -> arms::Runner {
    serde_json::from_value(json!({
        "name": "fixture", "directory": root, "manifest": manifest,
        "files": {}, "provenance": {}
    }))
    .unwrap()
}

fn delivered() -> Output {
    let mut output = Output::failed(1_000, "Custom runner failed");
    output.outcome = Outcome::Delivered;
    output.cause = None;
    output
}

fn sdk_session(root: &Path) {
    let output = root.join("output");
    fs::write(output.join("review.jsonl"), "review\n").unwrap();
    fs::write(
        output.join("sessions.json"),
        serde_json::to_vec(&json!([{
            "role":"review", "transcript_file":"review.jsonl",
            "started_at":"2026-11-14T15:00:00Z", "ended_at":"2026-11-14T15:00:02Z",
            "models":{"sdk-model":{"uncached_input_tokens":1,"cache_read_tokens":0,
                "cache_write_tokens":0,"output_tokens":1}}
        }]))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn timed_out_runner_after_provider_traffic_stays_failed() {
    let root = attempt();
    let mut recorder = Recorder::start(root.path()).unwrap();
    let runner = runner(
        root.path(),
        json!({"command": ["runner"], "providers": ["openai"]}),
    );
    let traffic = Accounting {
        provider_connections: 1,
        harness_invocations: vec![HarnessInvocation {
            harness: Harness::Codex,
            provider_connections: 1,
        }],
    };
    let mut output = Output::failed(30_000, "Trial timed out");
    collect(
        &mut recorder,
        &runner,
        &traffic,
        &mut output,
        false,
        &root.path().join("signal"),
    )
    .unwrap();
    assert!(output.outcome == Outcome::Failed, "{:?}", output.cause);
    assert_eq!(output.void_cause, None);
    assert_eq!(output.cause.as_deref(), Some("Trial timed out"));
}

#[test]
fn undeclared_runner_sending_provider_traffic_without_sessions_is_an_accounting_gap() {
    let root = attempt();
    let mut recorder = Recorder::start(root.path()).unwrap();
    let runner = runner(root.path(), json!({"command": ["runner"]}));
    let traffic = Accounting {
        provider_connections: 1,
        harness_invocations: Vec::new(),
    };
    let mut output = delivered();
    collect(
        &mut recorder,
        &runner,
        &traffic,
        &mut output,
        true,
        &root.path().join("signal"),
    )
    .unwrap();
    assert!(output.outcome == Outcome::Void, "{:?}", output.cause);
    assert_eq!(output.void_cause, Some(VoidCause::AccountingGap));
}

#[test]
fn harness_run_across_the_proxy_without_a_harness_session_is_an_accounting_gap() {
    let root = attempt();
    sdk_session(root.path());
    let mut recorder = Recorder::start(root.path()).unwrap();
    let runner = runner(root.path(), json!({"command": ["runner"]}));
    let traffic = Accounting {
        provider_connections: 2,
        harness_invocations: vec![HarnessInvocation {
            harness: Harness::Claude,
            provider_connections: 1,
        }],
    };
    let mut output = delivered();
    collect(
        &mut recorder,
        &runner,
        &traffic,
        &mut output,
        true,
        &root.path().join("signal"),
    )
    .unwrap();
    assert!(output.outcome == Outcome::Void, "{:?}", output.cause);
    assert_eq!(output.void_cause, Some(VoidCause::AccountingGap));
    assert!(
        output.cause.as_deref().unwrap().contains("claude"),
        "{:?}",
        output.cause
    );
}

#[test]
fn web_search_scan_skips_events_from_other_arm_builds() {
    let attempt = tempfile::tempdir().unwrap();
    fs::create_dir(attempt.path().join("session")).unwrap();
    let events = serde_json::to_string(&json!([
        {"type": "event_from_a_newer_arm_build"},
        {"type": "tool_call_started", "id": "s", "parent_id": null,
         "tool": "WebSearch", "input": {"query": "fix"}}
    ]))
    .unwrap();
    let line = json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{"attributes":[
        {"key":"thirdshift.events","value":{"stringValue": events}}]}]}]}]});
    fs::write(
        attempt.path().join("session/trial.logs.jsonl"),
        format!("{line}\n"),
    )
    .unwrap();
    assert!(web_search(attempt.path()).unwrap());
}

#[test]
fn custom_runner_results_require_a_confined_bounded_regular_file() {
    use std::os::unix::fs::symlink;
    use std::os::unix::process::ExitStatusExt;

    let process = std::process::Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    for case in [
        "regular",
        "at-limit",
        "too-long",
        "file-symlink",
        "directory-symlink",
        "directory",
        "missing",
    ] {
        let root = attempt();
        let directory = root.path().join("output");
        let path = directory.join("result.json");
        let mut bytes = br#"{"version":1,"outcome":"delivered"}"#.to_vec();
        if matches!(case, "at-limit" | "too-long") {
            let limit = usize::try_from(super::RESULT_LIMIT).unwrap();
            bytes.resize(limit + usize::from(case == "too-long"), b' ');
        }
        fs::write(&path, bytes).unwrap();
        match case {
            "file-symlink" => {
                fs::rename(&path, directory.join("original.json")).unwrap();
                symlink("original.json", &path).unwrap();
            }
            "directory-symlink" => {
                let moved = root.path().join("original-output");
                fs::rename(&directory, &moved).unwrap();
                symlink(moved, &directory).unwrap();
            }
            "directory" => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            "missing" => fs::remove_file(&path).unwrap(),
            _ => {}
        }
        let (output, reported) = super::result(&process, root.path(), 1);
        let accepted = matches!(case, "regular" | "at-limit");
        assert_eq!(reported, accepted, "{case}: {:?}", output.cause);
        assert!(
            output.outcome
                == if accepted {
                    Outcome::Delivered
                } else {
                    Outcome::Failed
                },
            "{case}: {:?}",
            output.cause
        );
        if case == "too-long" {
            assert!(output.cause.as_deref().unwrap().contains("longer than"));
        }
    }
}
