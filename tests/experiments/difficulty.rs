use super::*;

#[test]
fn recording_difficulty_refuses_different_setups_without_changing_manifests() {
    for (field, extra) in [
        ("commit", vec![]),
        ("Harness", vec!["--candidate-harness", "codex"]),
        ("Model", vec!["--candidate-model", "sonnet"]),
        ("Effort", vec!["--candidate-effort", "high"]),
    ] {
        let fixture = Fixture::new();
        fixture.scenario.agent_does(review_reports_prelude());
        let baseline = if field == "commit" {
            "main"
        } else {
            "candidate"
        };
        let mut args = vec!["--baseline", baseline, "--suite", "spec-review"];
        args.extend(extra);
        let run = fixture.run(&args);
        assert_eq!(run.code, Some(0), "{field}: {}", run.stderr);
        let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
        let id = experiment["id"].as_str().unwrap();
        let path = fixture
            .scenario
            .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
        let before = fs::read(&path).unwrap();
        let recorded = fixture
            .scenario
            .run(&["experiments", id, "--record-difficulty"]);
        assert_eq!(recorded.code, Some(1), "{field}: {}", recorded.stdout);
        assert!(
            recorded
                .stderr
                .contains("same commit, Harness, Model and Effort"),
            "{field}: {}",
            recorded.stderr
        );
        assert_eq!(fs::read(&path).unwrap(), before, "{field}");
    }
}

#[test]
fn recording_difficulty_refuses_an_unfinished_aa_experiment() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day"}}'
sleep 0.5
"#);
    let run = fixture.run(&["--baseline", "candidate", "--suite", "spec-review"]);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["state"], "paused");
    let path = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let before = fs::read(&path).unwrap();
    let recorded = fixture.scenario.run(&[
        "experiments",
        experiment["id"].as_str().unwrap(),
        "--record-difficulty",
    ]);
    assert_eq!(recorded.code, Some(1), "{}", recorded.stdout);
    assert!(
        recorded.stderr.contains("completed Experiment"),
        "{}",
        recorded.stderr
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn difficulty_leaves_void_trials_and_their_retries_out_of_both_counts() {
    let fixture = Fixture::with_session("spec-review", 3);
    fixture.scenario.agent_does(&format!(r#"
{reports}
case "$PWD" in *--candidate--1/*)
echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["login expired"]}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
;; esac
"#, reports = review_reports_prelude()));
    let run = fixture.run(&["--baseline", "candidate", "--suite", "spec-review"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(experiment["voids"]["candidate"]["count"], 3);
    let recorded = fixture.scenario.run(&[
        "experiments",
        experiment["id"].as_str().unwrap(),
        "--record-difficulty",
        "--json",
    ]);
    assert_eq!(recorded.code, Some(0), "{}", recorded.stderr);
    let recorded: Value = serde_json::from_str(&recorded.stdout).unwrap();
    assert_eq!(recorded[0]["difficulty"]["passes"], 5);
    assert_eq!(recorded[0]["difficulty"]["trials"], 5);
    assert_eq!(recorded[0]["measured"], false);
    assert_eq!(recorded[0]["discriminating"], false);
    let listed = fixture.scenario.run(&["evals"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed[0]["tasks"][0]["difficulty"]["trials"], 5);
    assert_eq!(listed[0]["tasks"][0]["measured"], false);
}

#[test]
fn evals_lists_unmeasured_tasks_and_the_fixed_discriminating_band() {
    let fixture = Fixture::new();
    super::discriminating::unmeasured(&fixture);
    let suite = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review");
    let original = fs::read_to_string(suite.join("tasks/fixture/task.toml")).unwrap();
    for (name, passes, trials, measured, discriminating) in [
        ("floor", 2, 20, true, false),
        ("lower", 3, 20, true, true),
        ("upper", 19, 20, true, true),
        ("ceiling", 20, 20, true, false),
        ("few", 4, 5, false, false),
        ("six", 3, 6, true, true),
        ("none", 0, 0, false, false),
    ] {
        let directory = suite.join("tasks").join(name);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("task.toml"), format!("{original}\n[difficulty]\npasses = {passes}\ntrials = {trials}\nexperiment = {:?}\ncommit = {:?}\nharness = \"claude\"\n", "a".repeat(64), fixture.candidate)).unwrap();
        let run = fixture.scenario.run(&["evals", "--json"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let listed: Value = serde_json::from_str(&run.stdout).unwrap();
        let task = listed[0]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["name"] == name)
            .unwrap();
        assert_eq!(task["measured"], measured, "{name}");
        assert_eq!(task["discriminating"], discriminating, "{name}");
        let unmeasured = listed[0]["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["name"] == "fixture")
            .unwrap();
        assert!(unmeasured["difficulty"].is_null());
        assert_eq!(unmeasured["measured"], false);
        assert_eq!(unmeasured["discriminating"], false);
    }
}

#[test]
fn approval_and_difficulty_share_the_suite_lock() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--baseline", "candidate", "--suite", "spec-review"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let id = experiment["id"].as_str().unwrap();
    let suite = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review");
    let path = suite.join("tasks/fixture/task.toml");
    let before = fs::read_to_string(&path)
        .unwrap()
        .replace("state = \"approved\"", "state = \"proposed\"");
    fs::write(&path, &before).unwrap();
    let lock = fs::File::create(suite.join("lock")).unwrap();
    lock.try_lock().unwrap();
    for args in [
        vec!["evals", "approve", "spec-review", "fixture"],
        vec!["experiments", id, "--record-difficulty"],
    ] {
        let run = fixture.scenario.run(&args);
        assert_eq!(run.code, Some(1), "{}", run.stdout);
        assert!(run.stderr.contains("is being updated"), "{}", run.stderr);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
    drop(lock);
    assert_eq!(
        fixture
            .scenario
            .run(&["experiments", id, "--record-difficulty"])
            .code,
        Some(0)
    );
    let approved = fixture
        .scenario
        .run(&["evals", "approve", "spec-review", "fixture"]);
    assert_eq!(approved.code, Some(0), "{}", approved.stderr);
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(manifest["state"].as_str(), Some("approved"));
    assert_eq!(manifest["difficulty"]["trials"].as_integer(), Some(2));
}

#[test]
fn difficulty_uses_the_latest_rechecked_score() {
    let fixture = Fixture::with_session("spec-review", 3);
    fixture.scenario.agent_does(review_reports_prelude());
    let grader = fixture.scenario.path("claude-script.sh.grader");
    fs::write(
        &grader,
        fs::read_to_string(&grader)
            .unwrap()
            .replace("borderline=False", "borderline=True"),
    )
    .unwrap();
    let run = fixture.run(&["--baseline", "candidate", "--suite", "spec-review"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let id = experiment["id"].as_str().unwrap();
    let trial = experiment["pending_rechecks"][0]["id"].as_str().unwrap();
    let checked = fixture.scenario.run(&[
        "recheck",
        id,
        trial,
        "fail",
        "--reason",
        "The report missed the finding.",
    ]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    let recorded = fixture
        .scenario
        .run(&["experiments", id, "--record-difficulty", "--json"]);
    assert_eq!(recorded.code, Some(0), "{}", recorded.stderr);
    let recorded: Value = serde_json::from_str(&recorded.stdout).unwrap();
    assert_eq!(recorded[0]["difficulty"]["passes"], 5);
    assert_eq!(recorded[0]["difficulty"]["trials"], 6);
    assert_eq!(recorded[0]["discriminating"], true);
}

#[test]
fn completed_aa_records_pooled_difficulty_for_every_task_and_replaces_old_readings() {
    let fixture = Fixture::with_session("code-review", 3);
    let suite = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/code-review");
    let original = fs::read_to_string(suite.join("tasks/fixture/task.toml")).unwrap();
    fs::write(
        suite.join("tasks/fixture/task.toml"),
        format!("# Keep this comment\n{original}"),
    )
    .unwrap();
    fs::create_dir_all(suite.join("tasks/second")).unwrap();
    for name in ["task.toml", "spec.md"] {
        fs::copy(
            suite.join("tasks/fixture").join(name),
            suite.join("tasks/second").join(name),
        )
        .unwrap();
    }
    fixture.scenario.agent_does(&format!(
        r#"
{reports}
case "$PWD" in
  *--candidate--1/*) printf 'missed\n' > "$reports/spec.md" ;;
  *) printf 'caught\n' > "$reports/spec.md" ;;
esac
"#,
        reports = review_reports_prelude()
    ));
    fs::write(fixture.scenario.path("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
json.dump({'scores': [dict(label=t['label'], passed='caught' in t['spec'], reason='Report checked.', wrong_finding=False, borderline=False) for t in packet['trials']]}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
"#).unwrap();
    let args = [
        "--baseline",
        "candidate",
        "--suite",
        "code-review",
        "--baseline-model",
        "opus",
        "--candidate-model",
        "opus",
        "--baseline-effort",
        "high",
        "--candidate-effort",
        "high",
    ];
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let id = experiment["id"].as_str().unwrap();
    let recorded = fixture
        .scenario
        .run(&["experiments", id, "--record-difficulty"]);
    assert_eq!(recorded.code, Some(0), "{}", recorded.stderr);
    for name in ["fixture", "second"] {
        assert!(
            recorded.stdout.contains(&format!("code-review/{name}")),
            "{}",
            recorded.stdout
        );
        let manifest = fs::read_to_string(suite.join(format!("tasks/{name}/task.toml"))).unwrap();
        let parsed: toml::Value = toml::from_str(&manifest).unwrap();
        let reading = &parsed["difficulty"];
        assert_eq!(reading["passes"].as_integer(), Some(5));
        assert_eq!(reading["trials"].as_integer(), Some(6));
        assert_eq!(reading["experiment"].as_str(), Some(id));
        assert_eq!(reading["commit"].as_str(), Some(fixture.candidate.as_str()));
        assert_eq!(reading["harness"].as_str(), Some("claude"));
        assert_eq!(reading["model"].as_str(), Some("opus"));
        assert_eq!(reading["effort"].as_str(), Some("high"));
    }
    assert!(recorded.stdout.contains("5/6"), "{}", recorded.stdout);
    assert!(
        recorded.stdout.contains("discriminating: true"),
        "{}",
        recorded.stdout
    );
    assert!(
        fs::read_to_string(suite.join("tasks/fixture/task.toml"))
            .unwrap()
            .starts_with("# Keep this comment\n")
    );
    let listed = fixture.scenario.run(&["evals", "list", "--json"]);
    assert_eq!(listed.code, Some(0), "{}", listed.stderr);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    for task in listed[0]["tasks"].as_array().unwrap() {
        assert_eq!(task["difficulty"]["passes"], 5);
        assert_eq!(task["difficulty"]["trials"], 6);
        assert_eq!(task["measured"], true);
        assert_eq!(task["discriminating"], true);
    }

    fixture.scenario.agent_does(&format!(
        "{}\nprintf 'missed\\n' > \"$reports/spec.md\"",
        review_reports_prelude()
    ));
    let again: Vec<_> = args
        .iter()
        .map(|arg| if *arg == "opus" { "sonnet" } else { *arg })
        .collect();
    let run = fixture.run(&again);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let next_id = experiment["id"].as_str().unwrap();
    assert_ne!(id, next_id);
    let recorded = fixture
        .scenario
        .run(&["experiments", next_id, "--record-difficulty", "--json"]);
    assert_eq!(recorded.code, Some(0), "{}", recorded.stderr);
    let recorded: Value = serde_json::from_str(&recorded.stdout).unwrap();
    for task in recorded.as_array().unwrap() {
        assert_eq!(task["difficulty"]["passes"], 0);
        assert_eq!(task["difficulty"]["trials"], 6);
        assert_eq!(task["difficulty"]["experiment"], next_id);
        assert_eq!(task["discriminating"], false);
        let manifest: toml::Value = toml::from_str(
            &fs::read_to_string(suite.join(format!(
                "tasks/{}/task.toml",
                task["task"].as_str().unwrap()
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["difficulty"]["experiment"].as_str(), Some(next_id));
        assert_eq!(manifest["difficulty"]["passes"].as_integer(), Some(0));
    }
}
