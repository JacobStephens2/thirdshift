use serde_json::{Value, json};

use super::*;
use crate::evals::experiment::{Bar, Conclusion};

fn completed_experiment(id: &str) -> Value {
    let setup = json!({"commit": "a", "harness": "claude", "model": null, "effort": null});
    json!({
        "format": 1, "id": id, "baseline": setup, "candidate": setup,
        "suites": [], "bar": Bar::default(), "state": "completed",
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
