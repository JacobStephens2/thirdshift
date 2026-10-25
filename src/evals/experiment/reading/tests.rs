use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Value, json};

use super::*;
use crate::evals::experiment::trial_results::files::observed::Observed;
use crate::evals::experiment::{Arm, Bar, trial_key};

fn write(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn stored(directory: &Path) -> Stored {
    let setup = json!({"commit": "a", "harness": "claude", "model": null, "effort": null});
    let mut stored: Stored = serde_json::from_value(json!({
        "format": 1, "id": "a".repeat(64), "baseline": setup, "candidate": setup,
        "suites": [], "bar": Bar::default(), "prepared": null,
        "state": "running", "created_at": "2026-10-24T10:00:00Z", "tasks": []
    }))
    .unwrap();
    stored.directory = directory.into();
    stored
}

fn output(attempt: &Path, milliseconds: u64, void: bool) -> Value {
    json!({"version": 1, "outcome": if void { "void" } else { "succeeded" },
        "timings": {"total_ms": milliseconds, "session_ms": milliseconds},
        "void_cause": if void { Some("usage_limit") } else { None },
        "cause": null, "session_id": null, "log": attempt.join("session/stdout.jsonl"),
        "model_evidence": null, "reports": {"standards": null, "spec": null}})
}

fn result(arm: &str, attempts: Vec<Value>, passed: bool) -> Value {
    json!({"suite": "code-review", "task": "fixture", "arm": arm, "trial": 0,
        "outcome": "succeeded", "attempts": attempts,
        "score": {"passed": passed, "wrong_finding": false, "checks": []}})
}

fn view(observation: Observation, stored: &Stored) -> Value {
    serde_json::to_value(observation.project(stored)).unwrap()
}

#[test]
fn one_capture_reuses_pending_terminal_and_active_facts_for_the_whole_view() {
    let root = tempfile::tempdir().unwrap();
    let mut stored = stored(root.path());
    let baseline = trial_key("code-review", "fixture", Arm::Baseline, 0);
    let candidate = trial_key("code-review", "fixture", Arm::Candidate, 0);
    let trial = root.path().join("trials").join(&baseline);
    let first = trial.join("attempt-old");
    let second = trial.join("attempt-new");
    let middle = trial.join("attempt-middle");
    let pending = result(
        "baseline",
        vec![
            output(&first, 60_000, true),
            output(&middle, 0, false),
            output(&second, 120_000, false),
        ],
        true,
    );
    write(&trial.join("pending.json"), &pending);
    let mut terminal_result = result("baseline", vec![], false);
    terminal_result["score"]["model"] = json!({"label": "blind", "passed": false,
        "reason": "model finding", "wrong_finding": true, "borderline": false});
    write(
        &root.path().join("results").join(format!("{baseline}.json")),
        &terminal_result,
    );
    write(&trial.join("active.json"), &json!({"started_at": "stale"}));
    for attempt in [&first, &middle, &second] {
        fs::create_dir_all(attempt.join("session")).unwrap();
        fs::write(attempt.join("session/stdout.jsonl"), "raw").unwrap();
    }
    write(
        &second.join("started.json"),
        &json!({"index": 7, "started_at": "override"}),
    );
    let running = root.path().join("trials").join(&candidate);
    write(
        &running.join("pending.json"),
        &result(
            "candidate",
            vec![output(&running.join("attempt-paused"), 30_000, true)],
            true,
        ),
    );
    write(
        &running.join("active.json"),
        &json!({"started_at": "sampled"}),
    );
    stored.rechecks.insert(
        baseline.clone(),
        serde_json::from_value(json!([
            {"passed": true, "reason": "human", "wrong_finding": null, "recorded_at": "now"}
        ]))
        .unwrap(),
    );
    let files = Observed::default();
    let changed = trial.join("pending.json");
    let results_folder = root.path().join("results");
    let sampled_terminal = results_folder.join(format!("{baseline}.json"));
    let later_terminal = results_folder.join(format!("{candidate}.json"));
    let sampled_active = running.join("active.json");
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "read"
            && (path == changed || path == sampled_terminal || path == sampled_active)
        {
            fs::write(path, "malformed after sampling").unwrap();
        }
        if operation == "entries" && path == results_folder {
            write(&later_terminal, &result("candidate", vec![], false));
        }
    }));
    let observation = Observation::capture(&files, &stored).unwrap();
    let counts = files.counts.borrow().clone();
    assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    assert_eq!(counts.get(&("read", trial.join("pending.json"))), Some(&1));
    assert_eq!(
        counts.get(&("entries", root.path().join("trials"))),
        Some(&1)
    );
    fs::remove_file(running.join("active.json")).unwrap();
    let captured = view(observation, &stored);
    assert_eq!(
        *files.counts.borrow(),
        counts,
        "projection performs no reads"
    );
    assert_eq!(captured["results"].as_array().unwrap().len(), 1);
    assert_eq!(captured["results"][0]["score"]["passed"], false);
    assert_eq!(
        captured["results"][0]["score"]["model"]["reason"],
        "model finding"
    );
    assert_eq!(
        captured["results"][0]["score"]["rechecks"][0]["reason"],
        "human"
    );
    assert_eq!(captured["minutes"]["baseline"], 3.0);
    assert_eq!(captured["minutes"]["candidate"], 0.5);
    assert_eq!(captured["voids"]["baseline"]["causes"]["usage_limit"], 1);
    assert_eq!(captured["unfinished_trials"].as_array().unwrap().len(), 1);
    assert_eq!(
        captured["running_trials"],
        json!([{"id": candidate, "started_at": "sampled"}])
    );
    assert_eq!(captured["sessions"][0]["attempt"], 0);
    assert_eq!(captured["sessions"][1]["attempt"], 1);
    assert_eq!(captured["sessions"][2]["attempt"], 7);
    write(&trial.join("pending.json"), &pending);
    write(
        &root.path().join("results").join(format!("{baseline}.json")),
        &terminal_result,
    );
    let fresh = view(Observation::capture(&Native, &stored).unwrap(), &stored);
    assert_eq!(fresh["results"].as_array().unwrap().len(), 2);
    assert_eq!(fresh["unfinished_trials"], json!([]));
    assert_eq!(fresh["running_trials"], json!([]));
}

#[test]
fn evidence_prefers_converted_logs_while_span_only_usage_is_captured_once() {
    let root = tempfile::tempdir().unwrap();
    let stored = stored(root.path());
    let key = trial_key("code-review", "fixture", Arm::Baseline, 0);
    let folder = root
        .path()
        .join("trials")
        .join(&key)
        .join("attempt-old/session");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("stdout.jsonl"), "raw").unwrap();
    let raw = view(Observation::capture(&Native, &stored).unwrap(), &stored);
    let id = raw["sessions"][0]["id"].clone();
    fs::write(folder.join("stdout.logs.jsonl"), "converted").unwrap();
    fs::write(folder.join("stdout.spans.jsonl"), "").unwrap();
    fs::write(
        folder.join("orphan.spans.jsonl"),
        concat!(
            "{\"resourceSpans\":[{\"scopeSpans\":[{\"spans\":[{",
            "\"spanId\":\"0000000000000001\",\"name\":\"turn 1\",\"attributes\":[",
            "{\"key\":\"gen_ai.usage.input_tokens\",\"value\":{\"intValue\":\"11\"}},",
            "{\"key\":\"gen_ai.usage.output_tokens\",\"value\":{\"intValue\":\"3\"}}]",
            "}]}]}]}\n"
        ),
    )
    .unwrap();
    let escaped = tempfile::tempdir().unwrap();
    fs::write(escaped.path().join("private.jsonl"), "secret").unwrap();
    std::os::unix::fs::symlink(
        escaped.path().join("private.jsonl"),
        folder.join("escape.jsonl"),
    )
    .unwrap();
    let escaped_attempt = folder
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("attempt-escape");
    fs::create_dir_all(&escaped_attempt).unwrap();
    std::os::unix::fs::symlink(escaped.path(), escaped_attempt.join("session")).unwrap();
    let files = Observed::default();
    let captured = Observation::capture(&files, &stored).unwrap();
    let counts = files.counts.borrow().clone();
    assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    assert_eq!(
        counts.get(&("open", folder.join("orphan.spans.jsonl"))),
        Some(&1)
    );
    fs::write(folder.join("orphan.spans.jsonl"), "malformed after capture").unwrap();
    let captured = view(captured, &stored);
    assert_eq!(captured["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(captured["sessions"][0]["id"], id);
    assert_eq!(captured["sessions"][0]["trace"], true);
    assert_eq!(captured["sessions"][0]["attempt"], 0);
    assert_eq!(
        captured["usage"]["baseline"]["models"]["unknown"]["input_tokens"],
        11
    );
    assert_eq!(
        captured["usage"]["baseline"]["models"]["unknown"]["output_tokens"],
        3
    );
    assert_eq!(*files.counts.borrow(), counts);
    assert!(Observation::capture(&Native, &stored).is_err());
}

#[test]
fn optional_absence_is_reused_until_a_new_capture() {
    let root = tempfile::tempdir().unwrap();
    let stored = stored(root.path());
    let key = trial_key("code-review", "fixture", Arm::Baseline, 0);
    let trial = root.path().join("trials").join(&key);
    let attempt = trial.join("attempt-new");
    fs::create_dir_all(&attempt).unwrap();
    let files = Observed::default();
    let publishing_trial = trial.clone();
    let publishing_attempt = attempt.clone();
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "read" && path == publishing_trial.join("pending.json") {
            write(
                path,
                &result(
                    "baseline",
                    vec![output(&publishing_attempt, 60_000, true)],
                    true,
                ),
            );
        } else if operation == "read" && path.file_name().unwrap() == "active.json" {
            write(path, &json!({"started_at": "later"}));
        } else if operation == "read" && path.file_name().unwrap() == "started.json" {
            write(path, &json!({"index": 9, "started_at": "later"}));
        } else if operation == "entries" && path == publishing_attempt.join("session") {
            fs::create_dir_all(path).unwrap();
            fs::write(path.join("stdout.jsonl"), "later").unwrap();
        }
    }));
    let captured = view(Observation::capture(&files, &stored).unwrap(), &stored);
    assert_eq!(captured["minutes"]["baseline"], 0.0);
    assert_eq!(captured["sessions"], json!([]));
    assert_eq!(captured["running_trials"], json!([]));
    assert_eq!(captured["unfinished_trials"], json!([]));
    assert!(files.counts.borrow().values().all(|count| *count == 1));
    let fresh = view(Observation::capture(&Native, &stored).unwrap(), &stored);
    assert_eq!(fresh["minutes"]["baseline"], 1.0);
    assert_eq!(fresh["sessions"][0]["attempt"], 9);
    assert_eq!(fresh["running_trials"][0]["started_at"], "later");
    assert_eq!(fresh["unfinished_trials"].as_array().unwrap().len(), 1);
}

#[test]
fn optional_records_removed_before_sampling_and_logs_removed_before_canonicalization_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let stored = stored(root.path());
    let key = trial_key("code-review", "fixture", Arm::Baseline, 0);
    let trial = root.path().join("trials").join(&key);
    let attempt = trial.join("attempt-new");
    write(
        &trial.join("pending.json"),
        &result("baseline", vec![], true),
    );
    write(&trial.join("active.json"), &json!({"started_at": "gone"}));
    write(
        &attempt.join("started.json"),
        &json!({"index": 3, "started_at": "gone"}),
    );
    fs::create_dir_all(attempt.join("session")).unwrap();
    fs::write(attempt.join("session/stdout.jsonl"), "gone").unwrap();
    let files = Observed::default();
    let trials = root.path().join("trials");
    let removed_trial = trial.clone();
    let removed_attempt = attempt.clone();
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "entries" && path == trials {
            fs::remove_file(removed_trial.join("pending.json")).unwrap();
            fs::remove_file(removed_trial.join("active.json")).unwrap();
            fs::remove_file(removed_attempt.join("started.json")).unwrap();
        } else if operation == "entries" && path == removed_attempt.join("session") {
            fs::remove_file(path.join("stdout.jsonl")).unwrap();
        }
    }));
    let captured = view(Observation::capture(&files, &stored).unwrap(), &stored);
    assert_eq!(captured["unfinished_trials"], json!([]));
    assert_eq!(captured["running_trials"], json!([]));
    assert_eq!(captured["sessions"], json!([]));
    assert!(files.counts.borrow().values().all(|count| *count == 1));
}

#[test]
fn corrupt_metadata_spans_and_permission_errors_remain_errors() {
    let root = tempfile::tempdir().unwrap();
    let stored = stored(root.path());
    let key = trial_key("code-review", "fixture", Arm::Baseline, 0);
    let trial = root.path().join("trials").join(&key);
    let attempt = trial.join("attempt-new");
    fs::create_dir_all(attempt.join("session")).unwrap();
    for path in [
        trial.join("pending.json"),
        trial.join("active.json"),
        attempt.join("started.json"),
        attempt.join("session/stdout.spans.jsonl"),
    ] {
        fs::write(&path, "malformed").unwrap();
        assert!(
            Observation::capture(&Native, &stored).is_err(),
            "{}",
            path.display()
        );
        fs::remove_file(&path).unwrap();
    }
    let files = Observed {
        read_error: Some((trial.join("pending.json"), io::ErrorKind::PermissionDenied)),
        ..Observed::default()
    };
    let error = Observation::capture(&files, &stored).err().unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn terminal_without_pending_keeps_zero_accounting_and_only_running_experiments_show_active_trials()
{
    let root = tempfile::tempdir().unwrap();
    let mut stored = stored(root.path());
    let key = trial_key("code-review", "fixture", Arm::Baseline, 0);
    write(
        &root.path().join("results").join(format!("{key}.json")),
        &result("baseline", vec![output(root.path(), 60_000, true)], false),
    );
    let candidate = trial_key("code-review", "fixture", Arm::Candidate, 0);
    write(
        &root
            .path()
            .join("trials")
            .join(&candidate)
            .join("active.json"),
        &json!({"started_at": "retained"}),
    );
    for state in [
        State::Prepared,
        State::Paused,
        State::Interrupted,
        State::Completed,
    ] {
        stored.state = state;
        let captured = view(Observation::capture(&Native, &stored).unwrap(), &stored);
        assert_eq!(captured["results"].as_array().unwrap().len(), 1);
        assert_eq!(captured["minutes"]["baseline"], 0.0);
        assert_eq!(captured["voids"]["baseline"]["count"], 0);
        assert_eq!(captured["running_trials"], json!([]));
    }
}
