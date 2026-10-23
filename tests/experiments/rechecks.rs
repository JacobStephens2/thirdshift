use super::*;

#[test]
fn decisive_score_waits_for_recheck_and_retains_both_scores_and_history() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&review_reports_prelude());
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
    fixture.scenario.agent_does(&review_reports_prelude());
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
    fixture.scenario.agent_does(&review_reports_prelude());
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
    fixture.scenario.agent_does(&review_reports_prelude());
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
    fixture.scenario.agent_does(&review_reports_prelude());
    assert_eq!(fixture.run(&["--prior", "0.01"]).code, Some(0));
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let path = fixture.scenario.path(&format!("home/.thirdshift/evals/experiments/{id}/experiment.json"));
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("rechecks");
    legacy.as_object_mut().unwrap().remove("pending_rechecks");
    legacy["verdict"].as_object_mut().unwrap().remove("provisional_verdict");
    legacy["verdict"]["verdict"] = json!("pass");
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert_eq!(fixture.experiment(), experiment);
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let check = fixture.scenario.run(&["recheck", id, trial, "pass", "--reason", "Rechecked the historical Score."]);
    assert_eq!(check.code, Some(0), "{}", check.stderr);
    assert_eq!(fixture.experiment()["verdict"]["verdict"], "pass");
}

#[test]
fn dropped_void_tasks_have_no_rechecks_and_refuse_human_scores() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("printf changed >> .thirdshift-issue.md");
    assert_eq!(fixture.run(&[]).code, Some(0));
    let experiment = fixture.experiment();
    assert_eq!(experiment["verdict"]["verdict"], "inconclusive");
    assert_eq!(experiment["pending_rechecks"], json!([]));
    let id = experiment["id"].as_str().unwrap();
    let results = fixture.scenario.path(&format!("home/.thirdshift/evals/experiments/{id}/results"));
    let trial = fs::read_dir(results).unwrap().next().unwrap().unwrap().path();
    let trial = trial.file_stem().unwrap().to_str().unwrap();
    let check = fixture.scenario.run(&["recheck", id, trial, "pass", "--reason", "Cannot revive a Void task."]);
    assert_ne!(check.code, Some(0));
    assert!(check.stderr.contains("Void Eval tasks cannot be rechecked"), "{}", check.stderr);
    assert_eq!(fixture.experiment(), experiment);
}
