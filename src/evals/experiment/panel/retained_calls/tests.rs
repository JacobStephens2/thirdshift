use std::io;

use serde_json::json;

use super::*;
use crate::evals::experiment::Bar;
use crate::evals::experiment::trial_results::files::observed::Observed;

fn description() -> Description {
    let setup = json!({"commit":"a", "harness":"claude", "model":null, "effort":null});
    let mut description: Description = serde_json::from_value(json!({
        "id":"fixture", "baseline":setup, "candidate":setup,
        "suites":["process"], "bar":Bar::default(), "prepared":null
    }))
    .unwrap();
    description.bar.trials_per_task.insert("process".into(), 2);
    description.bar.discriminating_tasks =
        Some(BTreeMap::from([("process".into(), vec!["judged".into()])]));
    description.bar.panel = Some(Panel {
        seats: [("claude", "a"), ("codex", "b")]
            .map(|(harness, model)| Seat {
                harness: harness.into(),
                model: model.into(),
                effort: None,
            })
            .into(),
        vote: Vote::Family,
        arm_family_rule: true,
        spending_limit: None,
        role: Role::Beside,
        families: 2,
        agreement: 0.8,
    });
    description
}

fn task(name: &str, session: &str) -> Task {
    serde_json::from_value(json!({"suite":"process", "name":name, "directory":"unused",
        "manifest":{"format":1, "state":"approved", "session":session,
            "repository":"unused", "base":"a", "change":"b", "criteria":"criteria.json",
            "inputs":{}, "trial":null}}))
    .unwrap()
}

// Historical calls need only decode structurally; fresh-response rubric and
// evidence validation deliberately does not apply to these fixtures.
fn call() -> Call {
    serde_json::from_value(json!({"shown_as_a":"baseline", "baseline":{
        "rubric":[], "merge_reason":"", "ready_to_merge":false}, "candidate":{
        "rubric":[], "merge_reason":"", "ready_to_merge":true},
        "reason":"", "preference":"candidate"}))
    .unwrap()
}

fn save(root: &Path, name: &str, trial: usize, seat: usize, order: usize) -> std::path::PathBuf {
    let path = call_path(
        root,
        &trial_results::pair_key("process", name, trial),
        seat,
        order,
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    store::write(&path, &call()).unwrap();
    path
}

fn finish(root: &Path, task: &Task, trial: usize, arm: Arm) {
    let identity = identities(task, trial)
        .into_iter()
        .find(|id| id.arm == arm)
        .unwrap();
    let directory = root.join("results");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join(format!("{}.json", identity.key())),
        "terminal marker",
    )
    .unwrap();
}

#[test]
fn a_later_corrupt_or_unreadable_order_is_not_hidden_by_missing_work() {
    let root = tempfile::tempdir().unwrap();
    let tasks = [task("judged", "process")];
    let description = description();
    let path = save(root.path(), "judged", 1, 1, 2);
    fs::write(&path, "malformed").unwrap();
    let error = History::capture(&Native, root.path(), &description, &tasks)
        .err()
        .unwrap();
    let text = format!("{error:#}");
    assert!(text.contains("process/judged #2"), "{text}");
    assert!(text.contains("seat 2, order 2"), "{text}");
    assert!(text.contains("expected value"), "{text}");
    let files = Observed {
        read_error: Some((path, io::ErrorKind::PermissionDenied)),
        ..Observed::default()
    };
    let error = History::capture(&files, root.path(), &description, &tasks)
        .err()
        .unwrap();
    let text = format!("{error:#}");
    assert!(text.contains("seat 2, order 2"), "{text}");
    assert!(text.contains("permission denied"), "{text}");
}

#[test]
fn absent_history_counts_unfinished_process_pairs_and_no_panel_reads_nothing() {
    let root = tempfile::tempdir().unwrap();
    let tasks = [
        task("measured", "process"),
        task("review", "code-review"),
        task("judged", "process"),
    ];
    let mut description = description();
    let files = Observed::default();
    let history = History::capture(&files, root.path(), &description, &tasks).unwrap();
    assert_eq!(history.remaining(), [(8, 4), (8, 4)]);
    assert!(history.executable().is_empty());
    assert_eq!(history.complete().count(), 0);
    assert_eq!(files.counts.borrow().len(), 16);
    description.bar.panel = None;
    let files = Observed::default();
    let history = History::capture(&files, root.path(), &description, &tasks).unwrap();
    assert!(history.remaining().is_empty());
    assert!(history.executable().is_empty());
    assert_eq!(history.complete().count(), 0);
    assert!(files.counts.borrow().is_empty());
    assert!(!root.path().join("panel").exists());
}

fn work_order(history: &History<'_>) -> Vec<(String, usize, usize, usize)> {
    history
        .executable()
        .into_iter()
        .map(|work| (work.task.name.clone(), work.trial, work.seat, work.order))
        .collect()
}

#[test]
fn partial_multi_seat_history_has_stable_counts_work_and_complete_report_inputs() {
    let root = tempfile::tempdir().unwrap();
    let tasks = [
        task("measured", "process"),
        task("review", "spec-review"),
        task("judged", "process"),
    ];
    let description = description();
    let retained = save(root.path(), "measured", 0, 0, 2);
    for seat in 0..2 {
        for order in 1..=2 {
            save(root.path(), "measured", 1, seat, order);
        }
    }
    save(root.path(), "judged", 0, 1, 1);
    for task in [&tasks[0], &tasks[2]] {
        for arm in [Arm::Baseline, Arm::Candidate] {
            finish(root.path(), task, 0, arm);
        }
    }
    finish(root.path(), &tasks[2], 1, Arm::Baseline);
    // Unexpected files, including calls for non-process tasks, stay ignored.
    let ignored = save(root.path(), "review", 0, 0, 1);
    fs::write(ignored, "malformed").unwrap();
    fs::write(root.path().join("panel/unexpected.json"), "malformed").unwrap();
    let files = Observed::default();
    let history = History::capture(&files, root.path(), &description, &tasks).unwrap();
    assert_eq!(history.remaining(), [(5, 3), (5, 3)]);
    let expected = [
        ("measured", 0, 0, 1),
        ("measured", 0, 1, 1),
        ("measured", 0, 1, 2),
        ("judged", 0, 0, 1),
        ("judged", 0, 0, 2),
        ("judged", 0, 1, 2),
    ]
    .map(|(name, trial, seat, order)| (name.to_owned(), trial, seat, order));
    assert_eq!(work_order(&history), expected);
    let reports: Vec<_> = history
        .complete()
        .map(|(task, trial, calls)| {
            assert_eq!(calls.len(), 2);
            assert!(calls.iter().all(|orders| orders.len() == 2));
            (task.name.as_str(), trial)
        })
        .collect();
    assert_eq!(reports, [("measured", 1)]);
    let reads = files.counts.borrow().clone();
    assert_eq!(reads.len(), 16);
    assert!(reads.values().all(|count| *count == 1));
    fs::remove_file(retained).unwrap();
    let missing = save(root.path(), "judged", 1, 1, 2);
    fs::write(missing, "malformed").unwrap();
    fs::remove_dir_all(root.path().join("results")).unwrap();
    assert_eq!(history.remaining(), [(5, 3), (5, 3)]);
    assert_eq!(work_order(&history), expected);
    assert_eq!(history.complete().count(), 1);
    assert_eq!(*files.counts.borrow(), reads);
    assert!(History::capture(&Native, root.path(), &description, &tasks).is_err());
}

#[test]
fn publication_advances_completion_only_after_a_durable_write_and_never_replaces_a_call() {
    let root = tempfile::tempdir().unwrap();
    let tasks = [task("judged", "process")];
    let mut description = description();
    description.bar.trials_per_task.insert("process".into(), 1);
    for arm in [Arm::Baseline, Arm::Candidate] {
        finish(root.path(), &tasks[0], 0, arm);
    }
    let mut history = History::capture(&Native, root.path(), &description, &tasks).unwrap();
    let work = history.executable();
    let saved = call_path(
        root.path(),
        &trial_results::pair_key("process", "judged", 0),
        0,
        1,
    );
    // Parent creation and temporary serialization succeed; atomic persist
    // fails because the destination is a directory.
    fs::create_dir_all(&saved).unwrap();
    assert!(history.publish(&work[0], call()).is_err());
    assert_eq!(history.remaining(), [(2, 1), (2, 1)]);
    assert_eq!(history.executable().len(), 4);
    assert_eq!(history.complete().count(), 0);
    fs::remove_dir(&saved).unwrap();
    for (index, item) in work.iter().enumerate() {
        history.publish(item, call()).unwrap();
        assert_eq!(history.executable().len(), 3 - index);
        assert_eq!(history.complete().count(), usize::from(index == 3));
    }
    assert_eq!(history.remaining(), [(0, 0), (0, 0)]);
    let before = fs::read(&saved).unwrap();
    assert!(history.publish(&work[0], call()).is_err());
    assert_eq!(fs::read(saved).unwrap(), before);
    let resumed = History::capture(&Native, root.path(), &description, &tasks).unwrap();
    assert_eq!(resumed.remaining(), [(0, 0), (0, 0)]);
    assert!(resumed.executable().is_empty());
    assert_eq!(resumed.complete().count(), 1);
}
