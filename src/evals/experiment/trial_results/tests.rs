use std::fs;

use serde_json::{Value, json};

use super::files::observed::Observed;
use super::*;
use crate::evals::experiment::trial_key;

fn output(void: bool) -> Output {
    serde_json::from_value(json!({
        "version": 1, "outcome": if void { "void" } else { "succeeded" },
        "timings": {"total_ms": 60_000, "session_ms": 60_000},
        "void_cause": if void { Some("usage_limit") } else { None },
        "cause": null, "session_id": null, "log": null,
        "model_evidence": null, "reports": {"standards": null, "spec": null}
    }))
    .unwrap()
}

fn score() -> grading::Score {
    serde_json::from_value(json!({"passed": true, "wrong_finding": false,
        "checks": [{"command": null, "passed": true, "reason": "mechanical"}]}))
    .unwrap()
}

fn published_trial() -> (tempfile::TempDir, String) {
    let root = tempfile::tempdir().unwrap();
    let result = retained(root.path(), vec![output(false)]);
    (root, result.key())
}

/// Reader fixtures use the retained format, without choosing lifecycle operations.
fn retained(root: &Path, attempts: Vec<Output>) -> TrialResult {
    let result = TrialResult {
        suite: "code-review".into(),
        task: "fixture".into(),
        arm: Arm::Baseline,
        trial: 0,
        outcome: attempts.last().unwrap().outcome,
        score: Some(score()),
        attempts,
    };
    let pending = root.join("trials").join(result.key());
    fs::create_dir_all(&pending).unwrap();
    fs::create_dir_all(root.join("results")).unwrap();
    store::write(&pending.join("pending.json"), &result).unwrap();
    store::write(
        &root.join("results").join(format!("{}.json", result.key())),
        &result,
    )
    .unwrap();
    result
}

#[test]
fn terminal_only_reads_ignore_unrelated_corrupt_pending_history() {
    let (root, _) = published_trial();
    let unrelated = root.path().join("trials/unrelated");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("pending.json"), "malformed").unwrap();
    fs::write(root.path().join("results/ignored.txt"), "malformed").unwrap();
    fs::create_dir(root.path().join("results/directory.json")).unwrap();
    let entries = terminal(root.path()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].result().key(),
        "9564a603e3b9de74ef4b54015a6dba5abb5c0b6897a2e6302dfe0512cdbce9e5--baseline--0"
    );
    assert_eq!(entries[0].result().attempts.len(), 1);
}

#[test]
fn model_updates_preserve_pending_accounting_and_rechecks_never_enter_raw_evidence() {
    let root = tempfile::tempdir().unwrap();
    let key = retained(root.path(), vec![output(true), output(false)]).key();
    let pending = root.path().join("trials").join(&key).join("pending.json");
    let mechanical = fs::read(&pending).unwrap();
    let selected = root.path().join("results/legacy-selected.json");
    fs::rename(
        root.path().join("results").join(format!("{key}.json")),
        &selected,
    )
    .unwrap();
    let mut entries = terminal(root.path()).unwrap();
    assert_eq!(
        serde_json::to_value(entries[0].target().unwrap()).unwrap(),
        "legacy-selected.json"
    );
    let assessment: BTreeMap<ScoreTarget, super::super::model_grader::Score> =
        serde_json::from_value(json!({
            "legacy-selected.json": {"label": "trial-1", "passed": false,
                "reason": "model finding", "wrong_finding": true, "borderline": true}
        }))
        .unwrap();
    apply_scores(&mut entries, &assessment).unwrap();
    let history: rechecks::History = serde_json::from_value(json!({key.clone(): [
        {"passed": true, "reason": "human", "wrong_finding": false, "recorded_at": "now"}
    ]}))
    .unwrap();
    let captured = capture(&Native, root.path()).unwrap();
    assert!(!captured.trials()[0].active_eligible());
    let progress = captured.into_progress(&history);
    assert_eq!(progress.minutes.baseline, 2.0);
    assert_eq!(progress.voids.baseline.count, 1);
    assert!(progress.unfinished_trials.is_empty());
    let effective_score = progress.results[0].score.as_ref().unwrap();
    assert!(!effective_score.passed);
    assert!(effective_score.effective().passed);
    assert_eq!(
        effective_score.model.as_ref().unwrap().reason,
        "model finding"
    );
    // Rendering and replay use separate raw entries, never an effective copy.
    let mut replay = terminal(root.path()).unwrap();
    assert!(
        replay[0]
            .result()
            .score
            .as_ref()
            .unwrap()
            .rechecks
            .is_empty()
    );
    apply_scores(&mut replay, &assessment).unwrap();
    assert_eq!(fs::read(&pending).unwrap(), mechanical);
    let raw = terminal(root.path()).unwrap();
    let raw_score = raw[0].result().score.as_ref().unwrap();
    assert!(!raw_score.passed);
    assert!(raw_score.rechecks.is_empty());
    let effective_results = effective(raw, &history);
    assert!(
        effective_results[0]
            .score
            .as_ref()
            .unwrap()
            .effective()
            .passed
    );
}

#[test]
fn capture_reuses_terminal_membership_and_absent_pending_until_the_next_observation() {
    let root = tempfile::tempdir().unwrap();
    let result = retained(root.path(), vec![output(true)]);
    let baseline = result.key();
    let pending = root
        .path()
        .join("trials")
        .join(&baseline)
        .join("pending.json");
    let saved = fs::read(&pending).unwrap();
    let mut candidate = serde_json::to_value(&result).unwrap();
    candidate["arm"] = json!("candidate");
    let candidate_key = trial_key("code-review", "fixture", Arm::Candidate, 0);
    let candidate_trial = root.path().join("trials").join(&candidate_key);
    fs::create_dir_all(&candidate_trial).unwrap();
    let candidate_pending = candidate_trial.join("pending.json");
    let later_terminal = root
        .path()
        .join("results")
        .join(format!("{candidate_key}.json"));
    let results_folder = root.path().join("results");
    let sampled_pending = pending.clone();
    let files = Observed::default();
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "entries" && path == results_folder {
            store::write(&later_terminal, &candidate).unwrap();
        } else if operation == "read" && path == candidate_pending {
            store::write(path, &candidate).unwrap();
        } else if operation == "read" && path == sampled_pending {
            fs::write(path, "corrupt after sampling").unwrap();
        }
    }));
    let captured = capture(&files, root.path()).unwrap();
    assert_eq!(captured.trials().len(), 2);
    assert!(!captured.trials()[0].active_eligible());
    assert!(captured.trials()[1].active_eligible());
    assert!(captured.trials()[1].pending.is_none());
    let counts = files.counts.borrow().clone();
    assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    assert_eq!(
        counts.get(&("entries", root.path().join("trials"))),
        Some(&1)
    );
    let progress = captured.into_progress(&BTreeMap::new());
    assert_eq!(progress.results.len(), 1);
    assert!(progress.unfinished_trials.is_empty());
    assert_eq!(progress.minutes.baseline, 1.0);
    assert_eq!(progress.minutes.candidate, 0.0);
    assert_eq!(*files.counts.borrow(), counts);
    assert!(capture(&Native, root.path()).is_err());
    fs::write(pending, saved).unwrap();
    let fresh = capture(&Native, root.path())
        .unwrap()
        .into_progress(&BTreeMap::new());
    assert_eq!(fresh.results.len(), 2);
    assert_eq!(fresh.minutes.candidate, 1.0);
    assert!(fresh.unfinished_trials.is_empty());
}

#[test]
fn saved_score_targets_resolve_only_selected_filenames_in_lexical_order() {
    let (root, key) = published_trial();
    let original = root.path().join("results").join(format!("{key}.json"));
    fs::copy(&original, root.path().join("results/zzz-selected.json")).unwrap();
    fs::rename(original, root.path().join("results/000-selected.json")).unwrap();
    let mut entries = terminal(root.path()).unwrap();
    let targets: Vec<_> = entries
        .iter()
        .map(|entry| entry.target().unwrap())
        .collect();
    assert_eq!(
        serde_json::to_value(targets).unwrap(),
        json!(["000-selected.json", "zzz-selected.json"])
    );
    let model = json!({"label": "trial-1", "passed": false, "reason": "model",
        "wrong_finding": true, "borderline": false});
    let invalid =
        serde_json::from_value(json!({"../outside.json": model, "zzz-selected.json": model}))
            .unwrap();
    let outside = root.path().join("outside.json");
    fs::write(&outside, "sentinel").unwrap();
    assert!(apply_scores(&mut entries, &invalid).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), "sentinel");
    assert!(
        terminal(root.path()).unwrap().iter().all(|entry| entry
            .result()
            .score
            .as_ref()
            .unwrap()
            .model
            .is_none())
    );
    let assessment =
        serde_json::from_value(json!({"000-selected.json": model, "zzz-selected.json": model}))
            .unwrap();
    apply_scores(&mut entries, &assessment).unwrap();
    assert!(
        terminal(root.path()).unwrap().iter().all(|entry| entry
            .result()
            .score
            .as_ref()
            .unwrap()
            .wrong_finding)
    );
}

#[test]
fn a_saved_assessment_replays_after_partial_terminal_application() {
    let (root, key) = published_trial();
    let original = root.path().join("results").join(format!("{key}.json"));
    let first = root.path().join("results/first.json");
    let second = root.path().join("results/second.json");
    fs::copy(&original, &second).unwrap();
    fs::rename(original, &first).unwrap();
    let model = json!({"label": "trial-1", "passed": false, "reason": "validated",
        "wrong_finding": true, "borderline": false});
    let saved = root.path().join("assessment.json");
    store::write(&saved, &json!({"first.json": model, "second.json": model})).unwrap();
    let scores = serde_json::from_slice(&fs::read(&saved).unwrap()).unwrap();
    let mut entries = terminal(root.path()).unwrap();
    let backup = root.path().join("second-backup.json");
    fs::rename(&second, &backup).unwrap();
    fs::create_dir(&second).unwrap();
    assert!(apply_scores(&mut entries, &scores).is_err());
    let applied: Value = serde_json::from_slice(&fs::read(&first).unwrap()).unwrap();
    let unapplied: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
    assert_eq!(applied["score"]["model"]["reason"], "validated");
    assert!(unapplied["score"]["model"].is_null());
    fs::remove_dir(&second).unwrap();
    fs::rename(backup, second).unwrap();
    drop(entries);
    let saved_scores = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    apply_scores(&mut terminal(root.path()).unwrap(), &saved_scores).unwrap();
    assert!(terminal(root.path()).unwrap().iter().all(|entry| {
        let score = entry.result().score.as_ref().unwrap();
        !score.passed && score.model.as_ref().unwrap().reason == "validated"
    }));
}
