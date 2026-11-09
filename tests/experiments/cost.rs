//! An Experiment shows its remaining Trials' cost before the first of them,
//! and asks on a terminal unless given `--yes` (#1482).

use super::*;

const QUESTION: &str = "Run these Trials? [y/N] ";

#[test]
fn declining_on_a_terminal_shows_the_cost_and_runs_no_trial() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");

    let declined = fixture.run_on_terminal(&["--timeout", "600"], &[(QUESTION, "n")]);

    assert_eq!(declined.code, Some(1), "{}", declined.stderr);
    for shown in [
        ": 2 Trials to run",
        "spec-review, Baseline arm (claude): 1 Trial, no earlier Trials to estimate from",
        "spec-review, Candidate arm (claude): 1 Trial, no earlier Trials to estimate from",
        "Worst case: 20 session-minutes (2 Trials × the Bar's 10-minute timeout)",
        "Experiment declined; no Trials were run",
    ] {
        assert!(
            declined.stderr.contains(shown),
            "{shown}: {}",
            declined.stderr
        );
    }
    assert!(
        !declined.stderr.contains("Estimate:"),
        "{}",
        declined.stderr
    );
    assert!(fixture.trial_calls().is_empty());
    // Nothing ran, so the new Experiment leaves no record behind.
    let listed = fixture.scenario.run(&["experiments", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&listed.stdout).unwrap(),
        json!([])
    );
}

#[test]
fn confirming_on_a_terminal_runs_the_trials() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");

    let confirmed = fixture.run_on_terminal(&[], &[(QUESTION, "y")]);

    assert_eq!(confirmed.code, Some(0), "{}", confirmed.stderr);
    assert_eq!(fixture.trial_calls().len(), 2);
}

#[test]
fn yes_and_a_non_terminal_stdin_skip_the_question() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let yes = fixture.run_on_terminal(&["--yes"], &[]);
    assert_eq!(yes.code, Some(0), "{}", yes.stderr);
    assert!(yes.stderr.contains("2 Trials to run"), "{}", yes.stderr);
    assert!(!yes.stderr.contains(QUESTION.trim()), "{}", yes.stderr);
    assert_eq!(fixture.trial_calls().len(), 2);

    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let piped = fixture.run(&[]);
    assert_eq!(piped.code, Some(0), "{}", piped.stderr);
    assert!(piped.stderr.contains("2 Trials to run"), "{}", piped.stderr);
    assert!(!piped.stderr.contains(QUESTION.trim()), "{}", piped.stderr);
    assert_eq!(fixture.trial_calls().len(), 2);
}

#[test]
fn the_estimate_uses_earlier_experiments_per_suite_and_harness() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let earlier = fixture.run(&[]);
    assert_eq!(earlier.code, Some(0), "{}", earlier.stderr);
    // Ten minutes for its completed Trial; a void one, however long, is not
    // a completed Trial.
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let results = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    for entry in fs::read_dir(results).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if result["arm"] == "baseline" {
            result["outcome"] = json!("void");
            result["attempts"][0]["outcome"] = json!("void");
            result["attempts"][0]["timings"]["total_ms"] = json!(6_000_000);
        } else {
            result["attempts"][0]["timings"]["total_ms"] = json!(600_000);
        }
        fs::write(&path, result.to_string()).unwrap();
    }

    let later = fixture.run_on_terminal(&["--baseline-harness", "codex"], &[(QUESTION, "n")]);

    assert_eq!(later.code, Some(1), "{}", later.stderr);
    assert!(
        later.stderr.contains(
            "spec-review, Baseline arm (codex): 1 Trial, no earlier Trials to estimate from"
        ),
        "{}",
        later.stderr
    );
    assert!(
        later
            .stderr
            .contains("spec-review, Candidate arm (claude): 1 Trial, estimate 10 session-minutes"),
        "{}",
        later.stderr
    );
    // One Arm has no history, so only the worst case totals.
    assert!(!later.stderr.contains("Estimate:"), "{}", later.stderr);

    let same = fixture.run_on_terminal(&["--candidate-model", "other-model"], &[(QUESTION, "n")]);
    assert_eq!(same.code, Some(1), "{}", same.stderr);
    assert!(
        same.stderr
            .contains("spec-review, Baseline arm (claude): 1 Trial, estimate 10 session-minutes"),
        "{}",
        same.stderr
    );
    assert!(
        same.stderr.contains(
            "thirdshift: Estimate: 20 session-minutes, from this machine's earlier Experiments"
        ),
        "{}",
        same.stderr
    );
}

#[test]
fn a_resume_shows_only_its_remaining_trials() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let run = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let directory = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}"));
    // Leave two of six Trials unfinished, as a pause would.
    let mut removed = 0;
    for entry in fs::read_dir(directory.join("results")).unwrap() {
        let path = entry.unwrap().path();
        if removed < 2 && path.to_str().unwrap().contains("--candidate--") {
            fs::remove_file(path).unwrap();
            removed += 1;
        }
    }
    let saved = directory.join("experiment.json");
    let mut experiment: Value = serde_json::from_slice(&fs::read(&saved).unwrap()).unwrap();
    experiment["state"] = json!("paused");
    fs::write(&saved, experiment.to_string()).unwrap();

    let resumed = fixture.run_on_terminal(&["--trials-per-task", "3"], &[(QUESTION, "n")]);

    assert_eq!(resumed.code, Some(1), "{}", resumed.stderr);
    assert!(
        resumed.stderr.contains(": 2 of 6 Trials remain"),
        "{}",
        resumed.stderr
    );
    assert!(
        resumed
            .stderr
            .contains("spec-review, Candidate arm (claude): 2 Trials, "),
        "{}",
        resumed.stderr
    );
    assert!(
        !resumed.stderr.contains("Baseline arm"),
        "{}",
        resumed.stderr
    );
    assert!(
        resumed
            .stderr
            .contains("Worst case: 60 session-minutes (2 Trials × the Bar's 30-minute timeout)"),
        "{}",
        resumed.stderr
    );
}

#[test]
fn the_estimate_counts_every_attempt_a_trial_spent() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let earlier = fixture.run(&[]);
    assert_eq!(earlier.code, Some(0), "{}", earlier.stderr);
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let results = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    // Each Trial retried once: five minutes per attempt, ten minutes spent,
    // which is what the Experiment's own Arm minutes report.
    for entry in fs::read_dir(results).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut attempt = result["attempts"][0].clone();
        attempt["timings"]["total_ms"] = json!(300_000);
        result["attempts"] = json!([attempt.clone(), attempt]);
        fs::write(&path, result.to_string()).unwrap();
    }

    // A fresh Experiment ID with both Harnesses unchanged, so the history applies.
    let later = fixture.run_on_terminal(&["--candidate-model", "other-model"], &[(QUESTION, "n")]);

    assert_eq!(later.code, Some(1), "{}", later.stderr);
    assert!(
        later
            .stderr
            .contains("spec-review, Baseline arm (claude): 1 Trial, estimate 10 session-minutes"),
        "{}",
        later.stderr
    );
}

#[test]
fn declining_leaves_no_experiment_in_the_list() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");

    let declined = fixture.run_on_terminal(&[], &[(QUESTION, "n")]);

    assert_eq!(declined.code, Some(1), "{}", declined.stderr);
    let listed = fixture.scenario.run(&["experiments", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed, json!([]), "{listed}");
}

#[test]
fn the_estimate_counts_every_attempt_of_a_retried_trial() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let earlier = fixture.run(&[]);
    assert_eq!(earlier.code, Some(0), "{}", earlier.stderr);
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let results = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    for entry in fs::read_dir(results).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // A Trial that went void once and succeeded on its retry: ten
        // session-minutes each attempt, twenty for the Trial.
        result["attempts"][0]["timings"]["total_ms"] = json!(600_000);
        let mut void = result["attempts"][0].clone();
        void["outcome"] = json!("void");
        void["void_cause"] = json!("usage_limit");
        result["attempts"] = json!([void, result["attempts"][0].clone()]);
        fs::write(&path, result.to_string()).unwrap();
    }

    let later = fixture.run_on_terminal(&["--candidate-model", "other-model"], &[(QUESTION, "n")]);

    assert_eq!(later.code, Some(1), "{}", later.stderr);
    assert!(
        later
            .stderr
            .contains("spec-review, Baseline arm (claude): 1 Trial, estimate 20 session-minutes"),
        "{}",
        later.stderr
    );
}
