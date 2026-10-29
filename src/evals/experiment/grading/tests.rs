use serde_json::{Value, json};

use super::*;

fn task(suite: &str, name: &str) -> Task {
    serde_json::from_value(json!({
        "suite": suite, "name": name, "directory": "/unused",
        "manifest": {
            "format": 1, "state": "approved", "session": "code-review",
            "repository": "fixture", "base": "a".repeat(40), "change": "b".repeat(40),
            "criteria": "catch the defect", "checks": [], "inputs": {}, "trial": null
        }
    }))
    .unwrap()
}

fn result(task: &Task, arm: Arm, passed: bool, wrong_finding: bool) -> TrialResult {
    TrialResult {
        suite: task.suite.clone(),
        task: task.name.clone(),
        arm,
        trial: 0,
        outcome: Outcome::Succeeded,
        score: Some(Score {
            passed,
            wrong_finding,
            borderline: false,
            model: None,
            checks: Vec::new(),
            rechecks: Vec::new(),
        }),
        attempts: Vec::new(),
    }
}

fn bar(suites: &[&str]) -> Bar {
    Bar {
        prior: 0.01,
        draws: 512,
        trials_per_task: suites.iter().map(|suite| ((*suite).into(), 1)).collect(),
        ..Bar::default()
    }
}

fn observation(tasks: &[Task], results: &[TrialResult], bar: &Bar) -> Value {
    let inputs = serde_json::to_value((tasks, results, bar)).unwrap();
    let first = serde_json::to_value(assess(tasks, results, bar).unwrap()).unwrap();
    let second = serde_json::to_value(assess(tasks, results, bar).unwrap()).unwrap();
    assert_eq!(first, second, "fixed-seed assessment repeats completely");
    assert_eq!(inputs, serde_json::to_value((tasks, results, bar)).unwrap());
    first
}

#[test]
fn assessment_returns_verdict_and_ordered_pending_without_changing_inputs() {
    let tasks = [task("code-review", "gain")];
    let results = [
        result(&tasks[0], Arm::Candidate, true, false),
        result(&tasks[0], Arm::Baseline, true, false),
    ];
    let assessment = observation(&tasks, &results, &bar(&["code-review"]));
    let verdict = &assessment[0];
    assert_eq!(verdict["verdict"], "waiting_on_rechecks");
    assert_eq!(verdict["provisional_verdict"], "pass");
    assert_eq!(verdict["borderline_trials"], 0);
    let suite = &verdict["suites"]["code-review"];
    assert_eq!(suite["verdict"], "pass");
    assert_eq!(suite["accuracy"]["difference_counts"], json!([0, 1, 0]));
    assert_eq!(
        suite["wrong_findings"]["difference_counts"],
        json!([0, 1, 0])
    );
    assert_eq!(suite["retained_tasks"], 1);
    assert_eq!(suite["dropped_tasks"], 0);
    assert_eq!(
        assessment[1],
        json!([{
            "id": results[0].key(), "suite": "code-review", "task": "gain",
            "arm": "candidate", "trial": 0, "borderline": false, "decisive": true,
            "decisive_flips": ["passed", "wrong_finding"],
            "decisive_verdicts": {"passed": "fail", "wrong_finding": "fail"}
        }])
    );
}

#[test]
fn decisive_flips_must_change_the_whole_experiment_across_suites() {
    let tasks = [task("code-review", "gain"), task("spec-review", "loss")];
    let mut results = vec![
        result(&tasks[0], Arm::Candidate, true, false),
        result(&tasks[1], Arm::Candidate, false, false),
        result(&tasks[1], Arm::Baseline, true, false),
        result(&tasks[0], Arm::Baseline, true, false),
    ];
    results[3].score.as_mut().unwrap().borderline = true;
    let isolated_results = [
        result(&tasks[0], Arm::Candidate, true, false),
        result(&tasks[0], Arm::Baseline, true, false),
    ];
    let isolated = observation(&tasks[..1], &isolated_results, &bar(&["code-review"]));
    assert_eq!(
        isolated[1][0]["decisive_flips"],
        json!(["passed", "wrong_finding"])
    );

    let assessment = observation(&tasks, &results, &bar(&["code-review", "spec-review"]));
    assert_eq!(assessment[0]["provisional_verdict"], "fail");
    assert_eq!(assessment[0]["verdict"], "waiting_on_rechecks");
    assert_eq!(assessment[0]["suites"]["code-review"]["verdict"], "pass");
    assert_eq!(assessment[0]["suites"]["spec-review"]["verdict"], "fail");
    assert_eq!(
        assessment[1],
        json!([
            {"id": results[1].key(), "suite": "spec-review", "task": "loss",
             "arm": "candidate", "trial": 0, "borderline": false, "decisive": true,
             "decisive_flips": ["passed"], "decisive_verdicts": {"passed": "pass"}},
            {"id": results[2].key(), "suite": "spec-review", "task": "loss",
             "arm": "baseline", "trial": 0, "borderline": false, "decisive": true,
             "decisive_flips": ["passed"], "decisive_verdicts": {"passed": "pass"}},
            {"id": results[3].key(), "suite": "code-review", "task": "gain",
             "arm": "baseline", "trial": 0, "borderline": true, "decisive": false,
             "decisive_flips": [], "decisive_verdicts": {}}
        ])
    );
}

#[test]
fn effective_human_scores_stay_projected_during_individual_wrong_finding_flips() {
    let tasks = [task("code-review", "gain")];
    let mut results = Vec::new();
    for trial in 0..3 {
        for arm in [Arm::Candidate, Arm::Baseline] {
            let candidate = matches!(arm, Arm::Candidate);
            let mut result = result(&tasks[0], arm, !candidate, true);
            result.trial = trial;
            let score = result.score.as_mut().unwrap();
            score.borderline = true;
            score.checks.push(Check {
                command: None,
                passed: true,
                reason: "mechanical evidence".into(),
            });
            score.model = Some(super::super::model_grader::Score {
                label: format!("trial-{trial}-{}", arm.name()),
                passed: !candidate,
                wrong_finding: true,
                borderline: true,
                reason: "model evidence".into(),
            });
            score.rechecks = serde_json::from_value(json!([
                {"passed": !candidate, "wrong_finding": true,
                 "reason": "first human decision", "recorded_at": "2026-01-01T00:00:00Z"},
                {"passed": candidate, "wrong_finding": false,
                 "reason": "human revision", "recorded_at": "2026-01-02T00:00:00Z"}
            ]))
            .unwrap();
            if candidate && trial == 0 {
                score.passed = true;
                score.wrong_finding = false;
                score.rechecks.clear();
            }
            results.push(result);
        }
    }
    let mut bar = bar(&["code-review"]);
    bar.trials_per_task.insert("code-review".into(), 3);
    let assessment = observation(&tasks, &results, &bar);
    let verdict = &assessment[0];
    assert_eq!(verdict["provisional_verdict"], "pass");
    assert_eq!(verdict["verdict"], "waiting_on_rechecks");
    assert_eq!(
        verdict["borderline_trials"], 6,
        "original borderline flags survive human rechecks"
    );
    let suite = &verdict["suites"]["code-review"];
    assert_eq!(
        suite["accuracy"]["difference_counts"],
        json!([0, 0, 0, 0, 0, 0, 1])
    );
    assert_eq!(
        suite["wrong_findings"]["difference_counts"],
        json!([0, 0, 0, 1, 0, 0, 0])
    );
    assert_eq!(
        suite["pass_cubed"],
        json!({"baseline": 0.0, "candidate": 1.0})
    );
    assert_eq!(suite["bar"]["trials_per_task"], 3);
    assert_eq!(
        assessment[1],
        json!([{
            "id": results[0].key(), "suite": "code-review", "task": "gain",
            "arm": "candidate", "trial": 0, "borderline": true, "decisive": true,
            "decisive_flips": ["wrong_finding"],
            "decisive_verdicts": {"wrong_finding": "fail"}
        }])
    );
}

#[test]
fn void_pairs_are_excluded_and_the_strict_dropout_guard_precedes_suite_decisions() {
    let tasks: Vec<_> = (0..10)
        .map(|i| task("code-review", &format!("task-{i}")))
        .collect();
    for void_tasks in [1, 2] {
        let mut results = Vec::new();
        for (index, task) in tasks.iter().enumerate() {
            for arm in [Arm::Baseline, Arm::Candidate] {
                let mut result = result(task, arm, matches!(arm, Arm::Candidate), false);
                if index < void_tasks {
                    // One Void Arm excludes its non-Void partner too.
                    if matches!(arm, Arm::Baseline) {
                        result.outcome = Outcome::Void;
                    }
                    result.score.as_mut().unwrap().borderline = true;
                }
                results.push(result);
            }
        }
        let assessment = observation(&tasks, &results, &bar(&["code-review"]));
        let verdict = &assessment[0];
        let suite = &verdict["suites"]["code-review"];
        assert_eq!(suite["retained_tasks"], 10 - void_tasks);
        assert_eq!(suite["dropped_tasks"], void_tasks);
        assert_eq!(suite["accuracy"]["verdict"], "pass");
        assert_eq!(suite["wrong_findings"]["verdict"], "pass");
        assert_eq!(
            suite["accuracy"]["difference_counts"],
            json!([0, 0, 10 - void_tasks])
        );
        assert_eq!(
            suite["wrong_findings"]["difference_counts"],
            json!([0, 10 - void_tasks, 0])
        );
        assert_eq!(verdict["borderline_trials"], void_tasks * 2);
        let pending = assessment[1].as_array().unwrap();
        for excluded in &results[..void_tasks * 2] {
            assert!(pending.iter().all(|p| p["id"] != excluded.key()));
        }
        if void_tasks == 1 {
            assert_eq!(
                verdict["provisional_verdict"], "pass",
                "exactly one tenth is allowed"
            );
            assert_eq!(verdict["verdict"], "waiting_on_rechecks");
            assert_eq!(pending.len(), 9);
        } else {
            assert_eq!(verdict["provisional_verdict"], "inconclusive");
            assert_eq!(verdict["verdict"], "inconclusive");
            assert!(
                pending.is_empty(),
                "no scalar flip can remove the dropout guard"
            );
        }
    }

    // The global guard also takes precedence over a failed independent suite.
    let mut tasks = tasks;
    tasks.push(task("spec-review", "loss"));
    let mut results = Vec::new();
    for (index, task) in tasks.iter().enumerate() {
        for arm in [Arm::Baseline, Arm::Candidate] {
            let mut result = result(task, arm, matches!(arm, Arm::Baseline), false);
            if index < 2 && matches!(arm, Arm::Candidate) {
                result.outcome = Outcome::Void;
            }
            results.push(result);
        }
    }
    let assessment = observation(&tasks, &results, &bar(&["code-review", "spec-review"]));
    assert_eq!(assessment[0]["suites"]["spec-review"]["verdict"], "fail");
    assert_eq!(assessment[0]["provisional_verdict"], "inconclusive");
    assert_eq!(assessment[0]["verdict"], "inconclusive");
    assert_eq!(assessment[1], json!([]));
}

#[test]
fn rejected_assessments_repeat_and_leave_all_inputs_unchanged_including_void_pairs() {
    let tasks = [task("code-review", "fixture")];
    for void in [false, true] {
        for invalid in ["missing Score", "incomplete pair", "invalid Bar"] {
            let mut results = vec![
                result(&tasks[0], Arm::Candidate, true, false),
                result(&tasks[0], Arm::Baseline, true, false),
            ];
            let mut bar = bar(&["code-review"]);
            if void {
                results[0].outcome = Outcome::Void;
            }
            let expected = match invalid {
                "missing Score" => {
                    results[0].score = None;
                    "Trial has no mechanical Score"
                }
                "incomplete pair" => {
                    results.pop();
                    "missing Trial results"
                }
                _ => {
                    bar.draws = 1;
                    "at least two interval draws are required"
                }
            };
            let inputs = serde_json::to_value((&tasks, &results, &bar)).unwrap();
            for _ in 0..2 {
                let error = assess(&tasks, &results, &bar)
                    .err()
                    .expect("assessment must fail");
                assert_eq!(error.to_string(), expected, "{invalid}, Void={void}");
                assert_eq!(
                    inputs,
                    serde_json::to_value((&tasks, &results, &bar)).unwrap()
                );
            }
        }
    }
}

#[test]
fn legacy_verdict_preserves_an_explicit_null_provisional_verdict() {
    let saved = json!({"verdict":"inconclusive","provisional_verdict":null,"suites":{},"borderline_trials":0});
    let decoded: Verdict = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), saved);
}
