use serde_json::{Value, json};

use super::*;
use crate::evals::experiment::Bar;
use crate::evals::experiment::deciders::{PeopleSetting, Role};

fn completed_experiment(id: &str) -> Value {
    let setup = json!({"commit": "a", "harness": "claude", "model": null, "effort": null});
    let bar = Bar {
        min_discriminating_tasks: Some(1),
        people: Some(PeopleSetting {
            role: Role::Counted,
            readers: vec!["reader".into()],
            sample: None,
            overlap: 0,
        }),
        ..Bar::default()
    };
    json!({
        "format": 1, "id": id, "baseline": setup, "candidate": setup,
        "suites": [], "bar": bar, "state": "completed",
        "created_at": "2026-10-24T10:00:00Z", "tasks": [],
        "verdict": {"verdict": "waiting_on_people", "provisional_verdict": "pass", "suites": {}}
    })
}

#[test]
fn saved_conclusion_ignores_display_artifacts_and_sees_later_decisions() {
    let root = tempfile::tempdir().unwrap();
    let id = "a".repeat(64);
    let directory = root.path().join(&id);
    fs::create_dir_all(directory.join("pairs")).unwrap();
    fs::write(directory.join("pairs/broken.json"), "invalid pair record").unwrap();
    let metadata = directory.join("experiment.json");
    let mut saved = completed_experiment(&id);
    write(&metadata, &saved).unwrap();

    assert!(matches!(
        read_conclusion(root.path(), &id).unwrap(),
        Conclusion::WaitingOnPeople
    ));

    saved["verdict"]["verdict"] = json!("pass");
    write(&metadata, &saved).unwrap();
    assert!(matches!(
        read_conclusion(root.path(), &id).unwrap(),
        Conclusion::Pass
    ));
}

#[test]
fn saved_conclusion_requires_matching_metadata_and_a_verdict() {
    let root = tempfile::tempdir().unwrap();
    let id = "a".repeat(64);
    let directory = root.path().join(&id);
    let metadata = directory.join("experiment.json");
    assert_eq!(
        read_conclusion(root.path(), "invalid")
            .unwrap_err()
            .to_string(),
        "invalid Experiment ID"
    );
    assert_eq!(
        read_conclusion(root.path(), &id).unwrap_err().to_string(),
        "Experiment not found"
    );
    fs::create_dir(&directory).unwrap();
    assert_eq!(
        read_conclusion(root.path(), &id).unwrap_err().to_string(),
        "Experiment not found"
    );

    fs::write(&metadata, "invalid metadata").unwrap();
    assert!(read_conclusion(root.path(), &id).is_err());

    let mut saved = completed_experiment(&"b".repeat(64));
    write(&metadata, &saved).unwrap();
    assert_eq!(
        read_conclusion(root.path(), &id).unwrap_err().to_string(),
        "Experiment ID does not match its folder"
    );

    saved["id"] = json!(id);
    saved["format"] = json!(2);
    write(&metadata, &saved).unwrap();
    assert_eq!(
        read_conclusion(root.path(), &id).unwrap_err().to_string(),
        "unsupported Experiment format"
    );

    saved["format"] = json!(1);
    saved.as_object_mut().unwrap().remove("verdict");
    write(&metadata, &saved).unwrap();
    assert_eq!(
        read_conclusion(root.path(), &id).unwrap_err().to_string(),
        "release Experiment has no Verdict"
    );
}

#[test]
fn resume_rejects_metadata_from_another_experiment() {
    let root = tempfile::tempdir().unwrap();
    let selected = "a".repeat(64);
    let unrelated = "b".repeat(64);
    let directory = root.path().join(&selected);
    fs::create_dir(&directory).unwrap();
    write(
        &directory.join("experiment.json"),
        &completed_experiment(&unrelated),
    )
    .unwrap();
    let requested: Description = serde_json::from_value(completed_experiment(&selected)).unwrap();
    match Stored::open(root.path(), requested, |_| panic!("resume chose new tasks")) {
        Err(error) => assert_eq!(error.to_string(), "Experiment ID does not match its folder"),
        Ok(Some(stored)) => panic!(
            "{selected} handed off {}",
            stored.into_completion().unwrap().id
        ),
        Ok(None) => panic!("resume skipped a completed Experiment"),
    }
}

#[test]
fn saved_legacy_conclusion_ignores_pending_display_upgrades() {
    let root = tempfile::tempdir().unwrap();
    let id = "a".repeat(64);
    let directory = root.path().join(&id);
    fs::create_dir_all(directory.join("results")).unwrap();
    fs::write(
        directory.join("results/broken.json"),
        "invalid Trial result",
    )
    .unwrap();
    let mut saved = completed_experiment(&id);
    saved["bar"] = json!(Bar::default());
    saved["verdict"]["verdict"] = json!("waiting_on_rechecks");
    saved["pending_rechecks"] = json!([{
        "id": "pending",
        "suite": "spec-review",
        "task": "fixture",
        "arm": "candidate",
        "trial": 0,
        "borderline": true,
        "decisive": true
    }]);
    write(&directory.join("experiment.json"), &saved).unwrap();
    assert!(matches!(
        read_conclusion(root.path(), &id).unwrap(),
        Conclusion::WaitingOnRechecks
    ));
}
