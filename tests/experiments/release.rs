use super::*;

#[test]
fn native_release_resumes_its_experiment_and_refuses_an_unattended_inconclusive_verdict() {
    let fixture = Fixture::new();
    let scenario = &fixture.scenario;
    scenario.user_config_is("[harness]\ndefault = 'claude'\n");
    scenario.launch_git(&[
        "config",
        "remote.origin.url",
        "https://github.com/JacobStephens2/thirdshift.git",
    ]);
    scenario.launch_git(&[
        "config",
        "--global",
        &format!("url.{}.insteadOf", scenario.path("origin.git").display()),
        "https://github.com/JacobStephens2/thirdshift.git",
    ]);
    scenario.launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    scenario.launch_git(&["push", "origin", "v0.1.0"]);
    scenario.commit_locally(
        "Cargo.toml",
        "[package]\nname = 'thirdshift'\nversion = '0.1.0'\n",
        "Manifest",
    );
    scenario.commit_locally(
        "Cargo.lock",
        "version = 4\n[[package]]\nname = 'thirdshift'\nversion = '0.1.0'\n",
        "Lockfile",
    );
    let candidate = scenario
        .launch_git(&["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    scenario.launch_git(&["push", "origin", "HEAD:main"]);
    let build = scenario.path(&format!("home/.thirdshift/evals/builds/{candidate}"));
    fs::create_dir_all(&build).unwrap();
    support::copy_executable(
        std::path::Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &build.join("thirdshift"),
    );
    let mut state = scenario.gh_state();
    state["repo"] = json!("JacobStephens2/thirdshift");
    state["runs"] = json!([{"databaseId":1,"workflow":"ci.yml","event":"push","branch":"main","headSha":candidate,"status":"completed","conclusion":"success","attempt":1}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(review_reports_prelude());
    let run = scenario.run_with_env(&["release", "0.2.0", "--with-experiment"], &NO_SANDBOX);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("unattended release has no terminal override"),
        "{}",
        run.stderr
    );
    let view = fixture.experiment();
    assert_eq!(view["candidate"]["commit"], candidate);
    assert_eq!(view["baseline"]["commit"], fixture.baseline);
    assert_eq!(view["verdict"]["verdict"], "inconclusive");
    let calls = fixture.trial_calls().len();
    assert!(calls > 0);
    scenario.commit_locally(
        "src/prompt.rs",
        "Newer content after interruption",
        "Newer main",
    );
    scenario.launch_git(&["push", "origin", "HEAD:main"]);
    let run = scenario.run_with_env(&["release", "--with-experiment"], &NO_SANDBOX);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("unattended release has no terminal override"),
        "{}",
        run.stderr
    );
    assert_eq!(
        fixture.trial_calls().len(),
        calls,
        "resumed release repeated completed Trials"
    );
    assert!(scenario.origin_git(&["tag", "--list", "v0.2.0"]).is_empty());
    let records = scenario
        .entries("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/release")
        .into_iter()
        .filter(|name| name.ends_with(".command.json"))
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    let overridden = scenario.run_on_terminal(
        &["release", "0.2.0", "--with-experiment"],
        &NO_SANDBOX,
        &[("release anyway? [y/N]", "y")],
    );
    assert!(
        overridden
            .stderr
            .contains("no release bump checks appeared"),
        "{}",
        overridden.stderr
    );
    assert!(
        !scenario
            .origin_git(&["branch", "--list", "release-0.2.0"])
            .is_empty()
    );
    assert_eq!(fixture.trial_calls().len(), calls);
}

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
    assert_eq!(experiment["name"], "Release candidate");
    assert_eq!(experiment["name_set"], false);
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
fn release_can_keep_the_published_baseline_when_recovering_an_unpublished_tag() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    fixture
        .scenario
        .launch_git(&["tag", "v0.2.0", &fixture.candidate]);
    fixture.scenario.agent_does(review_reports_prelude());

    let run = fixture.scenario.run_with_env(
        &[
            "experiment",
            "--release",
            "candidate",
            "--baseline",
            "v0.1.0",
            "--json",
        ],
        &NO_SANDBOX,
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.candidate);
    assert_eq!(experiment["suites"], json!(["spec-review"]));
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
