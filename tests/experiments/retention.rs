use super::*;

pub(super) fn attempts(fixture: &Fixture, experiment: &Value) -> Vec<PathBuf> {
    let trials = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/trials",
        experiment["id"].as_str().unwrap()
    ));
    fs::read_dir(trials)
        .unwrap()
        .flat_map(|trial| fs::read_dir(trial.unwrap().path()).unwrap())
        .map(|attempt| attempt.unwrap().path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("attempt-")
        })
        .collect()
}

#[test]
fn final_verdict_removes_all_repositories_and_keeps_experiment_and_session_output() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(experiment["pending_rechecks"], json!([]));
    assert_eq!(fixture.experiment(), experiment);
    let attempts = attempts(&fixture, &experiment);
    assert_eq!(attempts.len(), 2);
    for attempt in &attempts {
        assert!(
            !attempt.join("repository").exists(),
            "{}",
            attempt.display()
        );
        for file in [
            "request.json",
            "started.json",
            "stdout",
            "stderr",
            "scope",
            "result.json",
            "trace.spans.jsonl",
        ] {
            assert!(attempt.join(file).is_file(), "missing {file}");
        }
        assert!(attempt.join("session").is_dir());
    }
    for session in experiment["sessions"].as_array().unwrap() {
        let shown = fixture.scenario.run(&[
            "experiments",
            experiment["id"].as_str().unwrap(),
            "--session",
            session["id"].as_str().unwrap(),
        ]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let attempt = attempts
            .iter()
            .find(|attempt| {
                attempt.parent().unwrap().file_name().unwrap()
                    == session["trial_id"].as_str().unwrap()
            })
            .unwrap();
        assert_eq!(
            shown.stdout,
            format!(
                "{}\n",
                fs::read_to_string(attempt.join("session/trial-spec-review.logs.jsonl")).unwrap()
            )
        );
    }
    assert_eq!(fixture.run(&[]).code, Some(0));
    assert_eq!(fixture.experiment(), experiment);
}

#[test]
fn pruning_failures_warn_without_changing_saved_verdict_or_exit_status() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&[]).code, Some(0));
    let before = fixture.experiment();
    let attempts = attempts(&fixture, &before);
    // A non-directory at the disposable path fails even when run as root.
    fs::write(
        attempts[0].join("repository"),
        "Cannot remove as a directory",
    )
    .unwrap();
    fs::create_dir(attempts[1].join("repository")).unwrap();
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert!(
        resumed.stderr.contains("warning: could not prune"),
        "{}",
        resumed.stderr
    );
    assert!(resumed.stderr.contains("repository"), "{}", resumed.stderr);
    assert!(attempts[0].join("repository").is_file());
    assert!(!attempts[1].join("repository").exists());
    assert_eq!(fixture.experiment(), before);
    fs::remove_file(attempts[0].join("repository")).unwrap();
    fs::create_dir(attempts[0].join("repository")).unwrap();
    let retry = fixture.run(&[]);
    assert_eq!(retry.code, Some(0), "{}", retry.stderr);
    assert!(!attempts[0].join("repository").exists());
}

#[test]
fn cache_pruning_failures_warn_and_preserve_native_transcripts() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        r#"
{}
mkdir -p ../session/home/.codex/sessions ../session/home/.cache
printf bulk > ../session/home/.codex/cache
printf transcript > ../session/home/.codex/sessions/retained.jsonl
"#,
        review_reports_prelude()
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stderr.contains("warning: could not prune"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains(".codex/cache"), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert!(
        experiment["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|trial| trial["outcome"] == "succeeded")
    );
    for attempt in attempts(&fixture, &experiment) {
        assert!(!attempt.join("session/home/.cache").exists());
        assert_eq!(
            fs::read_to_string(attempt.join("session/home/.codex/sessions/retained.jsonl"))
                .unwrap(),
            "transcript"
        );
    }
}
