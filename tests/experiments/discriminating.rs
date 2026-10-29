use super::*;

#[test]
fn a_pre_trial_resume_uses_the_difficulty_that_selected_its_judged_tasks() {
    let fixture = Fixture::new();
    unmeasured(&fixture);
    fixture.scenario.agent_does(review_reports_prelude());
    let builds = fixture.scenario.path("home/.thirdshift/evals/builds");
    fs::remove_dir_all(&builds).unwrap();
    let cargo = fixture.scenario.path("bin/cargo");
    fs::write(&cargo, "#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let paused = fixture.run(&[]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let saved = fixture.experiment();
    assert_eq!(saved["state"], "paused");
    assert_eq!(saved["bar"]["min_discriminating_tasks"], 1);
    assert_eq!(
        saved["bar"]["discriminating_tasks"],
        json!({"spec-review": []})
    );
    let directory = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        saved["id"].as_str().unwrap()
    ));
    assert!(!directory.join("started").exists());
    assert!(fixture.trial_calls().is_empty());

    difficulty(&fixture, "fixture", 3, 6);
    let manifest_path = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&manifest_path).unwrap().replace(
        &format!("commit = {:?}", fixture.baseline),
        &format!("commit = {:?}", fixture.candidate),
    );
    fs::write(&manifest_path, manifest).unwrap();
    for commit in [&fixture.baseline, &fixture.candidate] {
        let build = builds.join(commit);
        fs::create_dir_all(&build).unwrap();
        support::copy_executable(
            std::path::Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            &build.join("thirdshift"),
        );
    }
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let saved: Value = serde_json::from_str(&resumed.stdout).unwrap();
    assert_eq!(
        saved["bar"]["discriminating_tasks"],
        json!({"spec-review": ["fixture"]})
    );
    assert_eq!(
        saved["verdict"]["suites"]["spec-review"]["counts"]["judged"],
        1
    );
    assert_eq!(
        saved["verdict"]["suites"]["spec-review"]["counts"]["other_setup"],
        1
    );
}

pub(super) fn unmeasured(fixture: &Fixture) {
    let path = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let mut manifest = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    manifest.remove("difficulty");
    fs::write(path, manifest.to_string()).unwrap();
}

pub(super) fn pre_selection_experiment(saved: &mut Value) {
    saved["bar"]
        .as_object_mut()
        .unwrap()
        .remove("discriminating_tasks");
    saved["bar"]
        .as_object_mut()
        .unwrap()
        .remove("min_discriminating_tasks");
    for suite in saved["verdict"]["suites"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        suite.as_object_mut().unwrap().remove("counts");
        suite.as_object_mut().unwrap().remove("measurement");
        suite.as_object_mut().unwrap().remove("advisory");
        suite["method"] = json!(
            "paired-dirichlet-v3; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles"
        );
    }
}

fn difficulty(fixture: &Fixture, task: &str, passes: usize, trials: usize) {
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/suites/spec-review/tasks/{task}/task.toml"
    ));
    let mut manifest: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    manifest.as_table_mut().unwrap().insert("difficulty".into(), toml::from_str(&format!(
        "passes = {passes}\ntrials = {trials}\nexperiment = {:?}\ncommit = {:?}\nharness = \"claude\"\n",
        "a".repeat(64), fixture.baseline
    )).unwrap());
    fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
}

#[test]
fn the_bar_fixes_discriminating_tasks_and_refuses_a_changed_list() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    difficulty(&fixture, "fixture", 3, 6);
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(
        saved["bar"]["discriminating_tasks"],
        json!({"spec-review": ["fixture"]})
    );
    let calls = fixture.trial_calls().len();
    difficulty(&fixture, "fixture", 4, 6);
    let unchanged_list = fixture.run(&[]);
    assert_eq!(unchanged_list.code, Some(0), "{}", unchanged_list.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&unchanged_list.stdout).unwrap()["verdict"],
        saved["verdict"]
    );
    difficulty(&fixture, "fixture", 0, 6);
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(1), "{}", resumed.stdout);
    assert!(
        resumed.stderr.contains("Bar cannot change"),
        "{}",
        resumed.stderr
    );
    assert_eq!(fixture.trial_calls().len(), calls);
}

#[test]
fn suite_counts_and_measurement_trials_include_unmeasured_and_out_of_band_tasks() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let tasks = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks");
    for (name, passes, trials) in [("other-setup", 3, 6), ("few", 3, 5), ("floor", 0, 6)] {
        fs::create_dir(tasks.join(name)).unwrap();
        for file in ["task.toml", "spec.md"] {
            fs::copy(
                tasks.join("fixture").join(file),
                tasks.join(name).join(file),
            )
            .unwrap();
        }
        difficulty(&fixture, name, passes, trials);
    }
    let other = tasks.join("other-setup/task.toml");
    let manifest = fs::read_to_string(&other).unwrap().replace(
        &format!("commit = {:?}", fixture.baseline),
        &format!("commit = {:?}", fixture.candidate),
    );
    fs::write(other, manifest).unwrap();
    let grader = fixture.scenario.path("claude-script.sh.grader");
    fs::write(
        &grader,
        fs::read_to_string(&grader)
            .unwrap()
            .replace("borderline=False", "borderline=True"),
    )
    .unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved = fixture.experiment();
    assert_eq!(fixture.trial_calls().len(), 8);
    assert_eq!(
        saved["bar"]["discriminating_tasks"],
        json!({"spec-review": ["fixture", "other-setup"]})
    );
    let suite = &saved["verdict"]["suites"]["spec-review"];
    assert_eq!(
        suite["counts"],
        json!({"judged": 2, "measurement_only": 2, "void_dropped": 0, "other_setup": 1})
    );
    let measurements = suite["measurement"].as_array().unwrap();
    assert_eq!(
        measurements
            .iter()
            .map(|task| task["task"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["few", "floor"]
    );
    assert!(
        measurements
            .iter()
            .all(|task| task["trials"].as_array().unwrap().len() == 2)
    );
    let pending = saved["pending_rechecks"].as_array().unwrap();
    assert_eq!(pending.len(), 4);
    assert!(
        pending
            .iter()
            .all(|trial| trial["task"] == "fixture" || trial["task"] == "other-setup")
    );
}

#[test]
fn pre_selection_experiments_keep_their_stored_bar_method_and_verdict() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut saved: Value = serde_json::from_str(&run.stdout).unwrap();
    pre_selection_experiment(&mut saved);
    saved["verdict"]["verdict"] = json!("fail");
    saved["verdict"]
        .as_object_mut()
        .unwrap()
        .remove("provisional_verdict");
    let id = saved["id"].as_str().unwrap().to_owned();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let mut metadata: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    metadata["bar"] = saved["bar"].clone();
    metadata["verdict"] = saved["verdict"].clone();
    fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    unmeasured(&fixture);
    let shown = fixture.experiment();
    assert_eq!(shown["bar"], saved["bar"]);
    assert_eq!(shown["verdict"], saved["verdict"]);
    assert_eq!(fs::read(&path).unwrap(), before);
    let calls = fixture.trial_calls().len();
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let resumed: Value = serde_json::from_str(&resumed.stdout).unwrap();
    assert_eq!(resumed["bar"], saved["bar"]);
    assert_eq!(resumed["verdict"], saved["verdict"]);
    assert_eq!(fixture.trial_calls().len(), calls);
}

#[test]
fn unmeasured_tasks_run_as_measurement_and_an_empty_judged_suite_is_inconclusive() {
    let fixture = Fixture::new();
    unmeasured(&fixture);
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(fixture.trial_calls().len(), 2);
    assert_eq!(
        saved["bar"]["discriminating_tasks"],
        json!({"spec-review": []})
    );
    assert_eq!(saved["verdict"]["verdict"], "inconclusive");
    assert_eq!(saved["pending_rechecks"], json!([]));
    let suite = &saved["verdict"]["suites"]["spec-review"];
    assert!(
        suite["method"]
            .as_str()
            .unwrap()
            .starts_with("paired-dirichlet-v4;")
    );
    assert_eq!(
        suite["counts"],
        json!({"judged": 0, "measurement_only": 1, "void_dropped": 0, "other_setup": 0})
    );
    assert_eq!(suite["measurement"][0]["task"], "fixture");
    let trials = suite["measurement"][0]["trials"].as_array().unwrap();
    assert_eq!(trials.len(), 2);
    assert!(trials.iter().all(|trial| trial["passed"] == true));
    assert_eq!(fixture.experiment()["verdict"], saved["verdict"]);
    let shown = fixture
        .scenario
        .run(&["experiments", saved["id"].as_str().unwrap()]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(
        shown
            .stdout
            .contains("0 judged, 1 measurement-only, 0 void-dropped, 0 other-setup"),
        "{}",
        shown.stdout
    );
    assert!(
        shown.stdout.contains("Measurement spec-review/fixture"),
        "{}",
        shown.stdout
    );
}
