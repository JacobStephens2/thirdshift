use super::*;

#[test]
fn release_names_all_approved_tasks_missing_trial_inputs_before_building() {
    assert_missing_trial_inputs_refused_before_builds(true);
}

#[test]
fn release_notes_a_baseline_without_the_trial_flag() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    let binary = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/builds/{}/thirdshift",
        fixture.baseline
    ));
    fs::write(
        binary,
        "#!/bin/sh\necho 'thirdshift: expected a GitHub Issue URL' >&2\nexit 2\n",
    )
    .unwrap();
    let run = fixture
        .scenario
        .run(&["experiment", "--release", "candidate", "--json"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let result: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(result["release_skip"], "baseline_without_trial");
    assert!(
        run.stderr.contains("v0.1.0 has no Trial flag"),
        "{}",
        run.stderr
    );
    assert!(fixture.trial_calls().is_empty());
    let listed = fixture.scenario.run(&["experiments", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&listed.stdout).unwrap(),
        json!([])
    );
}

#[test]
fn release_without_a_touched_suite_needs_no_experiment() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    let run = fixture
        .scenario
        .run(&["experiment", "--release", &fixture.baseline, "--json"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let result: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(result["release_skip"], "no_suites");
    assert!(fixture.trial_calls().is_empty());
    let listed = fixture.scenario.run(&["experiments", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&listed.stdout).unwrap(),
        json!([])
    );
}

#[test]
fn release_uses_the_last_tag_and_only_touched_suites_and_resumes() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    // main has already moved: its merge-base would select no suites here.
    fixture
        .scenario
        .launch_git(&["branch", "-f", "main", "candidate"]);
    fixture.scenario.agent_does(review_reports_prelude());
    let args = ["experiment", "--release", "candidate", "--json"];
    let first = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let experiment: Value = serde_json::from_str(&first.stdout).unwrap();
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.candidate);
    assert_eq!(experiment["suites"], json!(["spec-review"]));
    let calls = fixture.trial_calls().len();
    let resumed = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&resumed.stdout).unwrap(),
        experiment
    );
    assert_eq!(fixture.trial_calls().len(), calls);
}

#[test]
fn an_interrupted_release_resumes_the_same_experiment() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    fixture.scenario.agent_does(&format!(
        "touch '{}'\nsleep 60",
        fixture.scenario.path("release-started").display()
    ));
    let args = ["experiment", "--release", "candidate", "--json"];
    let mut running = fixture
        .scenario
        .run_until(&args, &NO_SANDBOX, "release-started");
    running.signal("INT");
    assert_eq!(running.finish().code, Some(1));
    let interrupted = fixture.experiment();
    assert_eq!(interrupted["state"], "interrupted");
    fixture.scenario.agent_does(review_reports_prelude());
    let resumed = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let completed = fixture.experiment();
    assert_eq!(completed["id"], interrupted["id"]);
    assert_eq!(completed["state"], "completed");
}
