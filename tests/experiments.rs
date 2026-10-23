mod support;

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use support::Scenario;

struct Fixture {
    scenario: Scenario,
    baseline: String,
    candidate: String,
}

impl Fixture {
    fn new() -> Self {
        let scenario = Scenario::new();
        for name in ["claude", "codex"] {
            let harness = scenario.path(&format!("bin/{name}"));
            let real = harness.canonicalize().unwrap();
            let fixed = scenario.path(&format!("versions/v1/{name}"));
            fs::create_dir_all(fixed.parent().unwrap()).unwrap();
            fs::copy(real, &fixed).unwrap();
            fs::remove_file(&harness).unwrap();
            std::os::unix::fs::symlink(fixed, &harness).unwrap();
        }
        let baseline = scenario
            .launch_git(&["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        scenario.launch_git(&["checkout", "-b", "candidate"]);
        fs::create_dir(scenario.launch_dir().join("src")).unwrap();
        scenario.commit_locally("src/prompt.rs", "candidate prompt", "Candidate");
        let candidate = scenario
            .launch_git(&["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        for commit in [&baseline, &candidate] {
            let build = scenario.path(&format!("home/.thirdshift/evals/builds/{commit}"));
            fs::create_dir_all(&build).unwrap();
            fs::copy(env!("CARGO_BIN_EXE_thirdshift"), build.join("thirdshift")).unwrap();
        }
        let suite = scenario.path("home/.thirdshift/evals/suites/spec-review");
        let task = suite.join("tasks/fixture");
        fs::create_dir_all(&task).unwrap();
        fs::write(
            suite.join("suite.toml"),
            r#"format = 1
trials_per_arm = 1
[dependencies]
factory_skills = []
session_prompts = ["src/prompt.rs"]
harness_defaults = []
model_defaults = []
effort_defaults = []
"#,
        )
        .unwrap();
        fs::write(
            task.join("spec.md"),
            "# Fixture Spec\n\nReview this invented change.\n",
        )
        .unwrap();
        fs::write(
            task.join("task.toml"),
            format!(
                r#"format = 1
state = "approved"
session = "spec-review"
repository = {:?}
base = "{baseline}"
change = "{baseline}"
criteria = "Report the invented finding."
[inputs]
".thirdshift-issue.md" = "spec.md"
[trial]
issue_url = "https://github.com/acme/widgets/issues/7"
base = "main"
branch = "issue-7"
pr_url = "https://github.com/acme/widgets/pull/1"
"#,
                scenario.launch_dir()
            ),
        )
        .unwrap();
        Self {
            scenario,
            baseline,
            candidate,
        }
    }

    fn run(&self, extra: &[&str]) -> support::RunResult {
        let mut args = vec!["experiment", "candidate", "--json"];
        args.extend_from_slice(extra);
        self.scenario.run(&args)
    }

    fn experiment(&self) -> Value {
        let listed = self.scenario.run(&["experiments", "--json"]);
        assert_eq!(listed.code, Some(0), "{}", listed.stderr);
        let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
        let id = listed[0]["id"].as_str().unwrap();
        let shown = self.scenario.run(&["experiments", id, "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        serde_json::from_str(&shown.stdout).unwrap()
    }
}

#[test]
fn mechanical_scores_determine_the_verdict_inbox_and_arm_minutes() {
    for (passing_arm, decision, group) in [
        ("candidate", "pass", "done"),
        ("baseline", "fail", "needs you"),
    ] {
        let fixture = Fixture::new();
        let task = fixture
            .scenario
            .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
        let manifest = fs::read_to_string(&task).unwrap();
        fs::write(&task, manifest.replace("[inputs]", &format!(
            "checks = [[\"sh\", \"-c\", 'grep -q {passing_arm} \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"']]\n[inputs]"
        ))).unwrap();
        fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
printf 'Standards report\n' > "$reports/standards.md"
case "$PWD" in
  *--candidate--*) printf 'candidate\n' > "$reports/spec.md" ;;
  *) printf 'baseline\n' > "$reports/spec.md" ;;
esac
"#);
        let run = fixture.run(&["--prior", "0.01"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let last = run.stdout.lines().last().unwrap();
        let experiment: Value = serde_json::from_str(last).unwrap();
        assert_eq!(experiment["verdict"]["verdict"], decision);
        assert!(
            experiment["verdict"]["suites"]["spec-review"]["accuracy"]["interval"]["lower"]
                .is_number()
        );
        for result in experiment["results"].as_array().unwrap() {
            assert_eq!(result["score"]["passed"], result["arm"] == passing_arm);
            assert!(
                result["score"]["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|check| check["command"].is_array())
            );
        }
        let id = experiment["id"].as_str().unwrap();
        let text = fixture.scenario.run(&["experiments", id]);
        assert!(text.stdout.contains("Baseline:"), "{}", text.stdout);
        assert!(text.stdout.contains("Candidate:"), "{}", text.stdout);
        for arm in ["baseline", "candidate"] {
            assert!(experiment["minutes"][arm].as_f64().unwrap() > 0.0);
        }
        assert_eq!(fixture.experiment(), experiment);
        let record = fixture
            .scenario
            .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
        assert_eq!(record["outcome"], decision);
        let status = fixture.scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], group);
        assert_eq!(fixture.run(&["--prior", "0.01"]).stdout, run.stdout);
    }
}

#[test]
fn changed_repositories_are_void_retried_and_excluded_from_the_verdict() {
    for mutation in [
        "printf changed >> .thirdshift-issue.md",
        "printf changed > invented-untracked.txt",
        "git -c user.name=Fixture -c user.email=fixture@example.invalid commit --allow-empty -m changed",
    ] {
        let fixture = Fixture::new();
        fixture.scenario.agent_does(mutation);
        let run = fixture.run(&[]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let experiment = fixture.experiment();
        assert_eq!(experiment["verdict"]["verdict"], "inconclusive");
        assert_eq!(
            experiment["verdict"]["suites"]["spec-review"]["dropped_tasks"],
            1
        );
        for result in experiment["results"].as_array().unwrap() {
            assert_eq!(result["outcome"], "void", "mutation: {mutation}");
            assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
            for attempt in result["attempts"].as_array().unwrap() {
                assert_eq!(attempt["void_cause"], "repository_changed");
            }
        }
        let status = fixture.scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], "needs you");
    }
}

#[test]
fn mechanical_check_output_stays_in_private_results() {
    let fixture = Fixture::new();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(
        &task,
        manifest.replace(
            "[inputs]",
            "checks = [[\"printf\", \"private-grader-evidence\"]]\n[inputs]",
        ),
    )
    .unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("private-grader-evidence"));
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    for log in fixture.scenario.log_files(folder, "log") {
        let log = fs::read_to_string(fixture.scenario.path(folder).join(log)).unwrap();
        assert!(
            !log.contains("private-grader-evidence"),
            "private Score entered factory logs"
        );
    }
}

#[test]
fn experiment_records_both_arms_the_bar_logs_stages_and_each_trial() {
    let fixture = Fixture::new();
    let result = fixture.run(&[
        "--baseline-model",
        "baseline-model",
        "--candidate-model",
        "candidate-model",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.lines().count(), 1);
    let experiment: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(experiment["state"], "completed");
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.candidate);
    assert_eq!(experiment["suites"], json!(["spec-review"]));
    assert_eq!(experiment["bar"]["margin"], -0.1);
    assert_eq!(experiment["bar"]["credible_level"], 0.95);
    assert_eq!(experiment["bar"]["trials_per_task"]["spec-review"], 1);
    assert_eq!(experiment["results"].as_array().unwrap().len(), 2);
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "succeeded");
        assert_eq!(
            result["score"]["passed"], false,
            "a review without reports fails"
        );
        assert!(PathBuf::from(result["attempts"][0]["log"].as_str().unwrap()).is_file());
    }
    assert_eq!(fixture.experiment(), experiment);
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    let record = fixture.scenario.command_record(folder);
    assert_eq!(record["kind"], "Experiment");
    assert_eq!(record["experiment"]["bar"], experiment["bar"]);
    assert_eq!(record["experiment"]["baseline"]["commit"], fixture.baseline);
    assert_eq!(
        record["experiment"]["candidate"]["commit"],
        fixture.candidate
    );
    let stages: Vec<_> = record["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(stages, ["Arm builds", "Eval tasks", "Trials"]);
    let activity = fs::read_to_string(
        fixture
            .scenario
            .path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("Experiment started: commands/experiment/"),
        "{activity}"
    );
    assert!(
        activity.contains("Experiment ended: inconclusive"),
        "{activity}"
    );
    let logs = fixture.scenario.log_files(folder, "log");
    assert_eq!(logs.len(), 1);
    let log = fs::read_to_string(fixture.scenario.path(folder).join(&logs[0])).unwrap();
    assert!(log.contains("Trials"), "{log}");
    let calls = fixture.scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().any(|c| {
        c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "baseline-model")
    }));
    assert!(calls.iter().any(|c| {
        c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "candidate-model")
    }));
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["kind"], "Experiment");
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(
        fixture
            .run(&[
                "--baseline-model",
                "baseline-model",
                "--candidate-model",
                "candidate-model"
            ])
            .code,
        Some(0)
    );
    assert_eq!(
        fixture.scenario.claude_calls().len(),
        2,
        "completed Experiment ran again"
    );
}

#[test]
fn stop_and_resume_preserve_finished_trials_and_refuse_a_changed_bar() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in
  *--0/*) exit 0 ;;
esac
touch '{}'
while true; do sleep 0.05; done
"#,
        fixture.scenario.path("started").display()
    ));
    let mut held = fixture.scenario.run_until(
        &[
            "experiment",
            "candidate",
            "--trials-per-task",
            "3",
            "--json",
        ],
        &[],
        "started",
    );
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    let stopped = fixture.scenario.run(&["stop", name, "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let stopped: Value = serde_json::from_str(&stopped.stdout).unwrap();
    assert_eq!(stopped["outcome"], "interrupted");
    let before = fixture.experiment();
    assert_eq!(before["state"], "interrupted");
    let finished = before["results"].as_array().unwrap();
    assert!(!finished.is_empty());
    assert!(finished.len() < 6);
    let calls_before = fixture.scenario.claude_calls().len();
    let changed = fixture.run(&["--trials-per-task", "3", "--seed", "1"]);
    assert_eq!(changed.code, Some(1));
    assert!(
        changed
            .stderr
            .contains("Bar cannot change after the first Trial"),
        "{}",
        changed.stderr
    );
    assert_eq!(fixture.scenario.claude_calls().len(), calls_before);
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let after = fixture.experiment();
    assert_eq!(after["state"], "completed");
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["results"].as_array().unwrap().len(), 6);
    for result in finished {
        assert!(
            after["results"].as_array().unwrap().contains(result),
            "finished Trial changed"
        );
    }
    assert_eq!(
        fixture.scenario.claude_calls().len(),
        calls_before + 6 - finished.len()
    );
    let activity = fs::read_to_string(
        fixture
            .scenario
            .path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("Experiment stopped from the CLI by"),
        "{activity}"
    );
}

#[test]
fn two_workers_interleave_the_arms_without_exceeding_two_active_trials() {
    let fixture = Fixture::new();
    let directory = fixture.scenario.path("concurrency");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("active"), "0").unwrap();
    fs::write(directory.join("maximum"), "0").unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
state='{}'
case "$PWD" in *--baseline--*) arm=baseline ;; *) arm=candidate ;; esac
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
active=$(cat "$state/active")
active=$((active + 1))
echo "$active" > "$state/active"
maximum=$(cat "$state/maximum")
if [ "$active" -gt "$maximum" ]; then echo "$active" > "$state/maximum"; fi
echo "$arm" >> "$state/starts"
rmdir "$state/lock"
sleep 0.3
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
active=$(cat "$state/active")
echo $((active - 1)) > "$state/active"
rmdir "$state/lock"
"#,
        directory.display()
    ));
    let result = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        fs::read_to_string(directory.join("maximum"))
            .unwrap()
            .trim(),
        "2"
    );
    assert_eq!(
        fs::read_to_string(directory.join("active")).unwrap().trim(),
        "0"
    );
    let starts = fs::read_to_string(directory.join("starts")).unwrap();
    let starts: Vec<_> = starts.lines().collect();
    assert_eq!(starts.len(), 6);
    assert_ne!(starts[0], starts[1], "{starts:?}");
    assert_eq!(starts.iter().filter(|arm| **arm == "baseline").count(), 3);
    assert_eq!(starts.iter().filter(|arm| **arm == "candidate").count(), 3);
}

#[test]
fn a_timed_out_trial_counts_as_failed_and_its_descendants_end() {
    let fixture = Fixture::new();
    let escaped = fixture.scenario.path("escaped-after-timeout");
    fixture.scenario.agent_does(&format!(
        r#"
(sleep 3; echo escaped > '{}') &
while true; do sleep 0.05; done
"#,
        escaped.display()
    ));
    let result = fixture.run(&["--timeout", "1"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "failed");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(result["attempts"][0]["cause"], "Trial timed out");
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert!(!escaped.exists(), "timeout left a descendant running");
}

#[test]
fn void_trials_retry_twice_in_fresh_repositories_and_keep_every_cause() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["connection refused"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    let result = fixture.run(&[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["results"].as_array().unwrap().len(), 2);
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "void");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
        for attempt in result["attempts"].as_array().unwrap() {
            assert_eq!(attempt["void_cause"], "session_unavailable");
        }
    }
    let calls = fixture.scenario.claude_calls();
    assert_eq!(calls.len(), 6);
    let repositories: std::collections::BTreeSet<_> = calls
        .iter()
        .map(|call| call["cwd"].as_str().unwrap())
        .collect();
    assert_eq!(repositories.len(), 6);
}

#[test]
fn explicit_suites_compare_one_build_on_distinct_harnesses_and_keep_fixed_paths() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        r#"
ln -sf /nonexistent/updated-claude '{}'
ln -sf /nonexistent/updated-codex '{}'
"#,
        fixture.scenario.path("bin/claude").display(),
        fixture.scenario.path("bin/codex").display()
    ));
    let result = fixture.scenario.run(&[
        "experiment",
        "main",
        "--baseline",
        "main",
        "--suite",
        "spec-review",
        "--baseline-harness",
        "claude",
        "--baseline-model",
        "opus",
        "--baseline-effort",
        "high",
        "--candidate-harness",
        "codex",
        "--candidate-model",
        "gpt-6.1-sol",
        "--candidate-effort",
        "max",
        "--trials-per-task",
        "3",
        "--json",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.baseline);
    assert_eq!(experiment["results"].as_array().unwrap().len(), 6);
    assert_eq!(fixture.scenario.claude_calls().len(), 3);
    assert_eq!(fixture.scenario.codex_calls().len(), 3);
    for (arm, harness, effort) in [
        ("baseline", "claude", "high"),
        ("candidate", "codex", "max"),
    ] {
        assert_eq!(experiment[arm]["harness"], harness);
        assert_eq!(experiment[arm]["effort"], effort);
        assert_eq!(
            experiment["prepared"][arm]["harness"]["path"],
            fixture
                .scenario
                .path(&format!("versions/v1/{harness}"))
                .to_str()
                .unwrap()
        );
    }
}

#[test]
fn distinct_suite_task_pairs_keep_distinct_trial_results() {
    let fixture = Fixture::new();
    let suites = fixture.scenario.path("home/.thirdshift/evals/suites");
    let source = suites.join("spec-review");
    for (suite, task) in [("alpha", "beta--gamma"), ("alpha--beta", "gamma")] {
        let target = suites.join(suite);
        let directory = target.join("tasks").join(task);
        fs::create_dir_all(&directory).unwrap();
        fs::copy(source.join("suite.toml"), target.join("suite.toml")).unwrap();
        fs::copy(
            source.join("tasks/fixture/task.toml"),
            directory.join("task.toml"),
        )
        .unwrap();
        fs::copy(
            source.join("tasks/fixture/spec.md"),
            directory.join("spec.md"),
        )
        .unwrap();
    }
    let result = fixture.run(&["--suite", "alpha", "--suite", "alpha--beta"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(experiment["state"], "completed");
    assert_eq!(fixture.scenario.claude_calls().len(), 4);
    assert_eq!(
        experiment["results"].as_array().unwrap().len(),
        4,
        "each suite/task/Arm Trial needs its own saved result: {experiment}"
    );
}

#[test]
fn resuming_a_cut_short_retry_keeps_the_bars_void_retry_budget() {
    let fixture = Fixture::new();
    let void = r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["connection refused"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#;
    fixture.scenario.agent_does_in_session(1, void);
    fixture.scenario.agent_does_in_session(2, void);
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in *--baseline--*) arm=baseline ;; *) arm=candidate ;; esac
touch '{root}/blocked-'"$arm"
while [ ! -f '{root}/blocked-baseline' ] || [ ! -f '{root}/blocked-candidate' ]; do sleep 0.01; done
touch '{root}/started'
while true; do sleep 0.05; done
"#,
        root = fixture.scenario.path("").display()
    ));
    let mut held =
        fixture
            .scenario
            .run_until(&["experiment", "candidate", "--json"], &[], "started");
    let status = fixture.scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    let stopped = fixture.scenario.run(&["stop", name, "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert_eq!(fixture.scenario.claude_calls().len(), 4);
    fixture.scenario.agent_does(void);
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(
        fixture.scenario.claude_calls().len(),
        8,
        "each Arm already completed one Void Trial attempt before Stop"
    );
    for result in fixture.experiment()["results"].as_array().unwrap() {
        assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
    }
}
