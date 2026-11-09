use super::*;

#[test]
fn the_minimum_is_fixed_before_trials_and_small_suites_are_advisory() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.scenario.run_with_env(
        &["experiment", "candidate", "--json", "--prior", "0.01"],
        &NO_SANDBOX,
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved = fixture.experiment();
    assert_eq!(saved["bar"]["min_discriminating_tasks"], 20);
    assert_eq!(saved["verdict"]["suites"]["spec-review"]["advisory"], true);
    assert_eq!(saved["verdict"]["verdict"], "inconclusive");
    assert_eq!(saved["pending_rechecks"], json!([]));
    let id = saved["id"].as_str().unwrap();
    let text = fixture.scenario.run(&["experiments", id]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains("spec-review: Advisory suite (pass)"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("1 judged, 0 measurement-only"),
        "{}",
        text.stdout
    );
    let calls = fixture.trial_calls().len();
    let resumed = fixture.run(&["--prior", "0.01", "--min-discriminating-tasks", "1"]);
    assert_eq!(resumed.code, Some(1), "{}", resumed.stdout);
    assert!(
        resumed.stderr.contains("Bar cannot change"),
        "{}",
        resumed.stderr
    );
    assert_eq!(fixture.trial_calls().len(), calls);
}

#[test]
fn a_custom_minimum_is_honoured_and_refused_on_resume_if_changed() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--prior", "0.01", "--min-discriminating-tasks", "1"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved = fixture.experiment();
    assert_eq!(saved["bar"]["min_discriminating_tasks"], 1);
    assert_eq!(saved["verdict"]["suites"]["spec-review"]["advisory"], false);
    assert_eq!(saved["verdict"]["provisional_verdict"], "pass");
    let resumed = fixture.run(&["--prior", "0.01", "--min-discriminating-tasks", "2"]);
    assert_eq!(resumed.code, Some(1), "{}", resumed.stdout);
    assert!(
        resumed.stderr.contains("Bar cannot change"),
        "{}",
        resumed.stderr
    );
}

#[test]
fn saved_discriminating_experiments_without_a_minimum_keep_their_bar_and_verdict() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&[]).code, Some(0));
    let mut saved = fixture.experiment();
    saved["bar"]
        .as_object_mut()
        .unwrap()
        .remove("min_discriminating_tasks");
    saved["verdict"]["suites"]["spec-review"]
        .as_object_mut()
        .unwrap()
        .remove("advisory");
    // Preserve a retained decision even when recalculation would disagree.
    saved["verdict"]["verdict"] = json!("fail");
    saved["verdict"]["provisional_verdict"] = json!("fail");
    saved["pending_rechecks"] = json!([]);
    let id = saved["id"].as_str().unwrap();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    // The CLI view omits private task manifests. Edit the retained record.
    let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    stored["bar"] = saved["bar"].clone();
    stored["verdict"] = saved["verdict"].clone();
    stored["pending_rechecks"] = json!([]);
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let shown = fixture.experiment();
    assert_eq!(shown["bar"], saved["bar"]);
    assert_eq!(shown["verdict"], saved["verdict"]);
    assert_eq!(fs::read(&path).unwrap(), before);
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let resumed: Value = serde_json::from_str(&resumed.stdout).unwrap();
    assert_eq!(resumed["bar"], saved["bar"]);
    assert_eq!(resumed["verdict"], saved["verdict"]);
}
