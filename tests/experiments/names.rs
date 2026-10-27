use super::*;

fn experiments(fixture: &Fixture, args: &[&str]) -> support::RunResult {
    let mut all = vec!["experiments"];
    all.extend_from_slice(args);
    fixture.scenario.run(&all)
}

fn rename(fixture: &Fixture, id: &str, name: &str) -> Value {
    let renamed = experiments(fixture, &[id, "--name", name, "--json"]);
    assert_eq!(renamed.code, Some(0), "{}", renamed.stderr);
    serde_json::from_str(&renamed.stdout).unwrap()
}

#[test]
fn a_named_experiment_shows_its_name_and_renames_never_refuse_its_resume() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(
        r#"echo '{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day"}}'
sleep 0.5
"#,
    );
    let args = ["--trials-per-task", "3", "--name", "  Prompt tweak  "];
    let paused = fixture.run(&args);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let before = fixture.experiment();
    assert_eq!(before["state"], "paused");
    assert_eq!(before["name"], "Prompt tweak");
    assert_eq!(before["name_set"], true);
    let id = before["id"].as_str().unwrap();

    let renamed = rename(&fixture, id, "Second try");
    assert_eq!(renamed["name"], "Second try");
    assert_eq!(renamed["state"], "paused");
    assert_eq!(fixture.experiment()["name"], "Second try");

    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let after = fixture.experiment();
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["state"], "completed");
    assert_eq!(after["name"], "Second try", "a resume keeps the name");

    for args in [vec![], vec![id]] {
        let shown = experiments(&fixture, &args);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        assert!(
            shown
                .stdout
                .starts_with(&format!("{id} · Second try · completed · ")),
            "{}",
            shown.stdout
        );
    }

    let cleared = rename(&fixture, id, "");
    assert_eq!(cleared["name"], "main (merge base) → candidate");
    assert_eq!(cleared["name_set"], false);
    assert_eq!(cleared["state"], "completed");
    let named = fixture.run(&["--trials-per-task", "3", "--name", "Done"]);
    assert_eq!(named.code, Some(0), "{}", named.stderr);
    assert_eq!(fixture.experiment()["name"], "Done");
}

#[test]
fn renaming_a_running_experiment_survives_its_later_writes() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        "touch '{}'\nwhile true; do sleep 0.05; done\n",
        fixture.scenario.path("started").display()
    ));
    let mut held = fixture.scenario.run_until(
        &["experiment", "candidate", "--json"],
        &NO_SANDBOX,
        "started",
    );
    let running = fixture.experiment();
    assert_eq!(running["state"], "running");
    let renamed = rename(&fixture, running["id"].as_str().unwrap(), "Live rename");
    assert_eq!(renamed["state"], "running");
    held.signal("INT");
    assert_eq!(held.finish().code, Some(1));
    let ended = fixture.experiment();
    assert_eq!(ended["state"], "interrupted");
    assert_eq!(ended["name"], "Live rename");
    assert_eq!(ended["name_set"], true);
}

#[test]
fn identical_arms_are_named_a_slash_a_at_their_ref() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
    let run = fixture.scenario.run_with_env(
        &[
            "experiment",
            "v0.1.0",
            "--baseline",
            "v0.1.0",
            "--suite",
            "spec-review",
            "--json",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["name"], "A/A at v0.1.0");
    assert_eq!(experiment["name_set"], false);
}

#[test]
fn empty_multiline_or_long_names_are_refused_before_any_build() {
    let fixture = Fixture::new();
    let long = "x".repeat(101);
    for (name, error) in [
        ("   ", "Experiment name must not be empty"),
        ("one\ntwo", "Experiment name must be one line"),
        (
            long.as_str(),
            "Experiment name must be at most 100 characters",
        ),
    ] {
        let refused = fixture.run(&["--name", name]);
        assert_ne!(refused.code, Some(0), "{}", refused.stderr);
        assert!(refused.stderr.contains(error), "{}", refused.stderr);
    }
    assert!(fixture.trial_calls().is_empty());
    assert!(
        !fixture
            .scenario
            .path("home/.thirdshift/evals/experiments")
            .exists(),
        "a refused name created an Experiment"
    );

    let completed = fixture.run(&[]);
    assert_eq!(completed.code, Some(0), "{}", completed.stderr);
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let refused = experiments(&fixture, &[&id, "--name", &long]);
    assert_ne!(refused.code, Some(0));
    assert!(
        refused
            .stderr
            .contains("Experiment name must be at most 100 characters"),
        "{}",
        refused.stderr
    );
    assert_eq!(fixture.experiment()["name_set"], false);
    let exact = "y".repeat(100);
    assert_eq!(rename(&fixture, &id, &exact)["name"], exact.as_str());
}
