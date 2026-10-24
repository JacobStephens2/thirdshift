use super::*;

#[test]
fn legacy_flip_verdicts_are_enriched_without_replacing_the_recorded_verdict() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let expected = fixture.experiment();
    assert!(!expected["pending_rechecks"].as_array().unwrap().is_empty());
    let id = expected["id"].as_str().unwrap();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    single_level_experiment(&mut saved);
    for pending in saved["pending_rechecks"].as_array_mut().unwrap() {
        pending.as_object_mut().unwrap().remove("decisive_verdicts");
    }
    fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let shown = fixture.experiment();
    assert_eq!(shown["bar"], saved["bar"]);
    assert_eq!(shown["verdict"], saved["verdict"]);
    assert_eq!(shown["pending_rechecks"], expected["pending_rechecks"]);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn batch_rechecks_record_the_reviewed_pending_scores_and_retain_history() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let script = fixture.scenario.path("claude-script.sh.grader");
    fs::write(
        &script,
        fs::read_to_string(&script)
            .unwrap()
            .replace("borderline=False", "borderline=True"),
    )
    .unwrap();
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    assert_eq!(experiment["pending_rechecks"].as_array().unwrap().len(), 2);
    let listed = fixture
        .scenario
        .run(&["recheck", id, "--pending", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed["experiment"], id);
    assert_eq!(listed["pending_rechecks"], experiment["pending_rechecks"]);
    let fingerprint = listed["fingerprint"].as_str().unwrap();
    assert_eq!(fingerprint.len(), 64);
    let text = fixture.scenario.run(&["recheck", id, "--pending"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains(fingerprint), "{}", text.stdout);
    for pending in listed["pending_rechecks"].as_array().unwrap() {
        for field in ["id", "suite", "task", "arm"] {
            assert!(text.stdout.contains(pending[field].as_str().unwrap()));
        }
    }
    assert!(
        text.stdout.contains("Trial 0 (borderline"),
        "{}",
        text.stdout
    );
    assert_eq!(fixture.experiment(), experiment, "listing changes nothing");
    let checked = fixture.scenario.run(&[
        "recheck",
        id,
        "--batch",
        fingerprint,
        "pass",
        "--reason",
        "Checked every report.",
        "--wrong-finding",
        "false",
        "--json",
    ]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    let checked: Value = serde_json::from_str(&checked.stdout).unwrap();
    assert_eq!(checked, fixture.experiment());
    assert_eq!(checked["pending_rechecks"], json!([]));
    assert_eq!(checked["verdict"]["verdict"], "pass");
    for result in checked["results"].as_array().unwrap() {
        assert_eq!(result["score"]["passed"], true, "original Score remains");
        assert_eq!(result["score"]["rechecks"].as_array().unwrap().len(), 1);
        let recheck = &result["score"]["rechecks"][0];
        assert_eq!(recheck["passed"], true);
        assert_eq!(recheck["wrong_finding"], false);
        assert_eq!(recheck["reason"], "Checked every report.");
        assert!(recheck["recorded_at"].is_string());
    }
    let pending_id = listed["pending_rechecks"][0]["id"].as_str().unwrap();
    let revised = fixture.scenario.run(&[
        "recheck",
        id,
        pending_id,
        "pass",
        "--reason",
        "Checked once more.",
    ]);
    assert_eq!(revised.code, Some(0), "{}", revised.stderr);
    let revised = fixture.experiment();
    let history = &revised["results"][0]["score"]["rechecks"];
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!(history[0]["reason"], "Checked every report.");
    assert_eq!(history[1]["reason"], "Checked once more.");
    assert_eq!(
        history[1]["wrong_finding"], false,
        "omission retains the flag"
    );
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    assert_eq!(fixture.experiment(), revised, "resume keeps batch history");
    assert_eq!(fixture.trial_calls().len(), 2);
    assert_eq!(fixture.grader_calls().len(), 1);
}

#[test]
fn stale_batch_fingerprints_leave_every_score_and_saved_byte_unchanged() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let listed = fixture
        .scenario
        .run(&["recheck", id, "--pending", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    let trial = listed["pending_rechecks"][0]["id"].as_str().unwrap();
    let changed = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "fail",
        "--reason",
        "Missed the finding.",
    ]);
    assert_eq!(changed.code, Some(0), "{}", changed.stderr);
    let changed = fixture.experiment();
    assert_eq!(changed["pending_rechecks"][0]["arm"], "baseline");
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let bytes = fs::read(&path).unwrap();
    let stale = fixture.scenario.run(&[
        "recheck",
        id,
        "--batch",
        listed["fingerprint"].as_str().unwrap(),
        "pass",
        "--reason",
        "Reviewed an older list.",
    ]);
    assert_eq!(stale.code, Some(1), "{}", stale.stdout);
    assert!(
        stale.stderr.contains("stale fingerprint"),
        "{}",
        stale.stderr
    );
    assert!(stale.stderr.contains("--pending"), "{}", stale.stderr);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fixture.experiment(), changed);
}

#[test]
fn invalid_concurrent_and_empty_batches_leave_the_experiment_unchanged() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let listed = fixture
        .scenario
        .run(&["recheck", id, "--pending", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    let fingerprint = listed["fingerprint"].as_str().unwrap();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let bytes = fs::read(&path).unwrap();
    for args in [
        vec!["recheck", id, "--batch", fingerprint, "pass"],
        vec![
            "recheck",
            id,
            "--batch",
            fingerprint,
            "pass",
            "--reason",
            "  ",
        ],
        vec![
            "recheck",
            id,
            "--batch",
            fingerprint,
            "maybe",
            "--reason",
            "Bad decision.",
        ],
        vec![
            "recheck",
            id,
            "--batch",
            fingerprint,
            "pass",
            "--reason",
            "Bad flag.",
            "--wrong-finding",
            "maybe",
        ],
        vec![
            "recheck",
            id,
            "--batch",
            "stale",
            "pass",
            "--reason",
            "Wrong fingerprint.",
        ],
        vec!["recheck", id, "--pending", "--reason", "Not a decision."],
    ] {
        let check = fixture.scenario.run(&args);
        assert_eq!(check.code, Some(1), "{args:?}: {}", check.stdout);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fixture.experiment(), experiment);
    }
    let lock = fs::File::open(path.parent().unwrap().join("lock")).unwrap();
    lock.try_lock().unwrap();
    let concurrent = fixture.scenario.run(&[
        "recheck",
        id,
        "--batch",
        fingerprint,
        "pass",
        "--reason",
        "Concurrent.",
    ]);
    assert_eq!(concurrent.code, Some(1));
    assert!(
        concurrent
            .stderr
            .contains("already running or being rechecked"),
        "{}",
        concurrent.stderr
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    drop(lock);
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        "--batch",
        fingerprint,
        "pass",
        "--reason",
        "Confirmed.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    let checked = fixture.experiment();
    let listed = fixture
        .scenario
        .run(&["recheck", id, "--pending", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed["pending_rechecks"], json!([]));
    let bytes = fs::read(&path).unwrap();
    let empty = fixture.scenario.run(&[
        "recheck",
        id,
        "--batch",
        listed["fingerprint"].as_str().unwrap(),
        "pass",
        "--reason",
        "Nothing to review.",
    ]);
    assert_eq!(empty.code, Some(1));
    assert!(
        empty.stderr.contains("no pending Scores"),
        "{}",
        empty.stderr
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(fixture.experiment(), checked);
}

#[test]
fn a_passed_recheck_moves_the_experiment_to_done() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["verdict"]["verdict"], "waiting_on_rechecks");
    let id = experiment["id"].as_str().unwrap();
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let checked = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "pass",
        "--reason",
        "I verified the written criteria.",
        "--json",
    ]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "pass");
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["group"], "done",
        "a completed passing Experiment belongs in Done: {status}"
    );
}

#[test]
fn decisive_score_waits_for_recheck_and_retains_both_scores_and_history() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["verdict"]["verdict"], "waiting_on_rechecks");
    assert_eq!(experiment["verdict"]["provisional_verdict"], "pass");
    let waiting = experiment["pending_rechecks"].as_array().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["arm"], "candidate");
    assert_eq!(waiting[0]["decisive"], true);
    assert_eq!(waiting[0]["borderline"], false);
    assert_eq!(
        waiting[0]["decisive_flips"],
        json!(["passed", "wrong_finding"])
    );
    assert_eq!(
        waiting[0]["decisive_verdicts"],
        json!({"passed": "fail", "wrong_finding": "fail"})
    );
    let id = experiment["id"].as_str().unwrap();
    let trial = waiting[0]["id"].as_str().unwrap();
    let text = fixture.scenario.run(&["experiments", id]);
    assert!(
        text.stdout.contains("waiting on rechecks"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains(trial), "{}", text.stdout);

    let recheck = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "pass",
        "--reason",
        "I verified the written criteria.",
        "--json",
    ]);
    assert_eq!(recheck.code, Some(0), "{}", recheck.stderr);
    let checked = fixture.experiment();
    assert_eq!(checked["verdict"]["verdict"], "pass");
    assert_eq!(checked["pending_rechecks"], json!([]));
    let candidate = checked["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["arm"] == "candidate")
        .unwrap();
    assert_eq!(candidate["score"]["passed"], true);
    assert_eq!(candidate["score"]["model"]["passed"], true);
    assert_eq!(candidate["score"]["rechecks"][0]["passed"], true);
    assert_eq!(
        candidate["score"]["rechecks"][0]["reason"],
        "I verified the written criteria."
    );
    assert!(candidate["score"]["rechecks"][0]["recorded_at"].is_string());

    let revised = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "fail",
        "--reason",
        "The report missed the required finding.",
        "--json",
    ]);
    assert_eq!(revised.code, Some(0), "{}", revised.stderr);
    let revised = fixture.experiment();
    assert_eq!(revised["verdict"]["provisional_verdict"], "fail");
    assert_eq!(revised["verdict"]["verdict"], "waiting_on_rechecks");
    assert_eq!(revised["pending_rechecks"][0]["arm"], "baseline");
    let candidate = revised["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["arm"] == "candidate")
        .unwrap();
    assert_eq!(
        candidate["score"]["passed"], true,
        "Grader's Score is immutable"
    );
    assert_eq!(candidate["score"]["rechecks"].as_array().unwrap().len(), 2);
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    assert_eq!(fixture.experiment(), revised, "resume keeps rechecks");
}

#[test]
fn borderline_scores_wait_even_when_their_flip_cannot_change_the_verdict() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    fs::write(fixture.scenario.path("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
json.dump({'scores': [dict(label=trial['label'], passed=True, reason='Needs a human check.', wrong_finding=False, borderline=True) for trial in packet['trials']]}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
"#).unwrap();
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let pending = experiment["pending_rechecks"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    let baseline = pending
        .iter()
        .find(|trial| trial["arm"] == "baseline")
        .unwrap();
    assert_eq!(baseline["borderline"], true);
    assert_eq!(baseline["decisive"], false);
    assert_eq!(baseline["decisive_flips"], json!([]));
    assert_eq!(baseline["decisive_verdicts"], json!({}));
    let id = experiment["id"].as_str().unwrap();
    let candidate = pending
        .iter()
        .find(|trial| trial["arm"] == "candidate")
        .unwrap();
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        candidate["id"].as_str().unwrap(),
        "pass",
        "--reason",
        "Confirmed.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    let checked = fixture.experiment();
    assert_eq!(checked["verdict"]["verdict"], "waiting_on_rechecks");
    assert_eq!(checked["pending_rechecks"].as_array().unwrap().len(), 1);
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        baseline["id"].as_str().unwrap(),
        "fail",
        "--reason",
        "Missed a required finding.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "pass");
    assert_eq!(fixture.experiment()["pending_rechecks"], json!([]));
}

#[test]
fn wrong_finding_rechecks_change_the_verdict_and_keep_the_flag_on_revision() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let candidate = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        candidate,
        "pass",
        "--reason",
        "It caught the required defect but invented another.",
        "--wrong-finding",
        "true",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    let checked = fixture.experiment();
    assert_eq!(checked["verdict"]["provisional_verdict"], "fail");
    assert_eq!(
        checked["verdict"]["suites"]["spec-review"]["accuracy"]["verdict"],
        "pass"
    );
    assert_eq!(
        checked["verdict"]["suites"]["spec-review"]["wrong_findings"]["verdict"],
        "fail"
    );
    let baseline = checked["pending_rechecks"][0]["id"].as_str().unwrap();
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        baseline,
        "pass",
        "--reason",
        "No wrong finding.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "fail");
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        candidate,
        "pass",
        "--reason",
        "Revised reason, same finding.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "fail");
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        candidate,
        "pass",
        "--reason",
        "The finding was valid after all.",
        "--wrong-finding",
        "false",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "pass");
    assert_eq!(
        fixture.trial_calls().len(),
        2,
        "rechecks never rerun Trials"
    );
    assert_eq!(
        fixture.grader_calls().len(),
        1,
        "rechecks never rerun Graders"
    );
}

#[test]
fn invalid_or_concurrent_rechecks_leave_the_experiment_unchanged() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    for args in [
        vec!["recheck", id, trial, "pass"],
        vec!["recheck", id, trial, "fail", "--reason", "  "],
        vec![
            "recheck",
            id,
            "../experiment.json",
            "pass",
            "--reason",
            "Bad Trial.",
        ],
        vec![
            "recheck",
            "../outside",
            trial,
            "pass",
            "--reason",
            "Bad Experiment.",
        ],
        vec!["recheck", id, trial, "maybe", "--reason", "Bad decision."],
        vec![
            "recheck",
            id,
            trial,
            "pass",
            "--reason",
            "Bad flag.",
            "--wrong-finding",
            "maybe",
        ],
    ] {
        let check = fixture.scenario.run(&args);
        assert_ne!(check.code, Some(0), "{args:?}: {}", check.stdout);
        assert_eq!(fixture.experiment(), experiment);
    }
    let lock = fs::File::create(
        fixture
            .scenario
            .path(&format!("home/.thirdshift/evals/experiments/{id}/lock")),
    )
    .unwrap();
    lock.try_lock().unwrap();
    let check = fixture
        .scenario
        .run(&["recheck", id, trial, "pass", "--reason", "Concurrent."]);
    assert_ne!(check.code, Some(0));
    assert!(
        check.stderr.contains("already running or being rechecked"),
        "{}",
        check.stderr
    );
    assert_eq!(fixture.experiment(), experiment);
}

#[test]
fn completed_experiments_saved_before_rechecks_acquire_the_pending_gate() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for pending in legacy["pending_rechecks"].as_array_mut().unwrap() {
        pending.as_object_mut().unwrap().remove("decisive_verdicts");
    }
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(
        fixture.experiment(),
        experiment,
        "older pending lists acquire flip Verdicts"
    );
    legacy.as_object_mut().unwrap().remove("rechecks");
    legacy.as_object_mut().unwrap().remove("pending_rechecks");
    legacy["verdict"]
        .as_object_mut()
        .unwrap()
        .remove("provisional_verdict");
    legacy["verdict"]["verdict"] = json!("pass");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(fixture.experiment(), experiment);
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "pass",
        "--reason",
        "Rechecked the historical Score.",
    ]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "pass");
}

#[test]
fn dropped_void_tasks_have_no_rechecks_and_refuse_human_scores() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .agent_does("printf changed >> .thirdshift-issue.md");
    assert_eq!(fixture.run(&[]).code, Some(0));
    let experiment = fixture.experiment();
    assert_eq!(experiment["verdict"]["verdict"], "inconclusive");
    assert_eq!(experiment["pending_rechecks"], json!([]));
    let id = experiment["id"].as_str().unwrap();
    let results = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    let trial = fs::read_dir(results)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let trial = trial.file_stem().unwrap().to_str().unwrap();
    let check = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "pass",
        "--reason",
        "Cannot revive a Void task.",
    ]);
    assert_ne!(check.code, Some(0));
    assert!(
        check.stderr.contains("Void Eval tasks cannot be rechecked"),
        "{}",
        check.stderr
    );
    assert_eq!(fixture.experiment(), experiment);
}
