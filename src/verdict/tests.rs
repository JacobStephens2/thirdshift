use super::{Bar, Decision, Score, TaskScores, Verdict, judge};

fn task(baseline_passes: usize, candidate_passes: usize, trials: usize) -> TaskScores {
    let arm = |passes| {
        (0..trials)
            .map(|i| Score {
                passed: i < passes,
                wrong_finding: false,
            })
            .collect()
    };
    TaskScores {
        baseline: arm(baseline_passes),
        candidate: arm(candidate_passes),
        dropped_as_void: false,
    }
}

fn with_levels(bar: &Bar, pass: f64, fail: f64) -> Bar {
    let mut json = serde_json::to_value(bar).unwrap();
    json.as_object_mut().unwrap().remove("credible_level");
    json["pass_level"] = pass.into();
    json["fail_level"] = fail.into();
    serde_json::from_value(json).unwrap()
}

#[test]
fn pass_and_fail_levels_judge_separately_with_an_inconclusive_between() {
    let bar = with_levels(&Bar::default(), 0.8, 0.95);
    let equal = vec![task(3, 3, 3); 10];
    let pass = judge(&equal, &bar).unwrap();
    assert_eq!(pass.verdict, Decision::Pass);
    assert_eq!(
        judge(&equal, &Bar::default()).unwrap().verdict,
        Decision::Inconclusive
    );
    assert_eq!(
        judge(&vec![task(3, 0, 3); 40], &bar).unwrap().verdict,
        Decision::Fail
    );
    assert_eq!(
        judge(&[task(3, 0, 3), task(0, 3, 3), task(3, 3, 3)], &bar)
            .unwrap()
            .verdict,
        Decision::Inconclusive
    );
}

#[test]
fn fail_takes_precedence_and_a_pass_interval_must_reach_zero() {
    let mut slight_loss = vec![task(3, 3, 3); 29];
    slight_loss.push(task(3, 2, 3));
    let bar = with_levels(&Bar::default(), 0.95, 0.1);
    let accuracy = super::judge_without_predictions(&slight_loss, &bar)
        .unwrap()
        .accuracy;
    assert!(accuracy.intervals.pass().lower > bar.margin);
    assert!(accuracy.intervals.pass().upper >= 0.0);
    assert!(accuracy.intervals.fail().upper < 0.0);
    assert_eq!(accuracy.verdict, Decision::Fail);

    let mut loss = vec![task(3, 3, 3); 27];
    loss.extend(vec![task(3, 2, 3); 3]);
    let bar = with_levels(&Bar::default(), 0.5, 0.999999);
    let accuracy = super::judge_without_predictions(&loss, &bar)
        .unwrap()
        .accuracy;
    assert!(accuracy.intervals.pass().lower > bar.margin);
    assert!(accuracy.intervals.pass().upper < 0.0);
    assert!(accuracy.intervals.fail().upper >= 0.0);
    assert_eq!(accuracy.verdict, Decision::Inconclusive);
}

#[test]
fn the_smallest_identical_code_review_suite_passing_the_default_bar_is_thirteen_tasks() {
    let bar = Bar::default();
    for count in 1..=13 {
        let verdict = super::judge_without_predictions(&vec![task(3, 3, 3); count], &bar).unwrap();
        assert_eq!(
            verdict.verdict,
            if count == 13 {
                Decision::Pass
            } else {
                Decision::Inconclusive
            },
            "{count} Eval tasks: accuracy {:?}, wrong findings {:?}",
            verdict.accuracy.intervals,
            verdict.wrong_findings.intervals,
        );
    }
}

#[test]
fn each_prediction_level_changes_the_chance_of_settling_an_inconclusive_metric() {
    let mut bar = Bar::default();
    bar.prediction.max_additional_tasks = 20;
    let scores = [task(3, 0, 3), task(0, 3, 3), task(3, 3, 3)];
    let probability = |bar: &Bar| {
        let verdict = judge(&scores, bar).unwrap();
        assert_eq!(verdict.accuracy.verdict, Decision::Inconclusive);
        verdict.accuracy.more_tasks.unwrap().clear_probabilities
    };
    let reference = probability(&bar);
    assert_ne!(probability(&with_levels(&bar, 0.8, 0.95)), reference);
    assert_ne!(probability(&with_levels(&bar, 0.95, 0.8)), reference);
}

#[test]
fn both_intervals_use_the_same_draws_as_the_legacy_method() {
    let scores = vec![task(3, 3, 3); 40];
    let separate =
        super::judge_without_predictions(&scores, &with_levels(&Bar::default(), 0.8, 0.95))
            .unwrap();
    for (level, expected) in [
        (0.8, separate.accuracy.intervals.pass()),
        (0.95, separate.accuracy.intervals.fail()),
    ] {
        let legacy_bar = Bar {
            levels: super::Levels::Legacy {
                credible_level: level,
            },
            ..Bar::default()
        };
        let legacy = super::judge_without_predictions(&scores, &legacy_bar).unwrap();
        assert_eq!(legacy.accuracy.intervals.pass(), expected);
        let json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(json["bar"]["credible_level"], level);
        assert!(json["bar"].get("pass_level").is_none());
        assert!(json["accuracy"]["interval"]["lower"].is_number());
        assert!(
            json["method"]
                .as_str()
                .unwrap()
                .starts_with("paired-dirichlet-v1;")
        );
        let decoded: Verdict = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), json);
    }
}

#[test]
fn equal_differences_keep_uncertainty_and_repeat_with_the_seed() {
    let scores = vec![task(3, 3, 3); 40];
    let bar = Bar::default();
    let verdict = judge(&scores, &bar).unwrap();
    assert!(verdict.accuracy.intervals.pass().lower < 0.0);
    assert!(verdict.accuracy.intervals.pass().upper > 0.0);
    assert_eq!(verdict, judge(&scores, &bar).unwrap());
}

#[test]
fn verdict_rules_include_a_sure_loss_inside_the_margin() {
    let bar = Bar::default();
    assert_eq!(
        judge(&vec![task(3, 0, 3); 40], &bar)
            .unwrap()
            .accuracy
            .verdict,
        Decision::Fail
    );
    assert_eq!(
        judge(&vec![task(0, 3, 3); 40], &bar)
            .unwrap()
            .accuracy
            .verdict,
        Decision::Pass
    );

    let mut small_loss = vec![task(3, 2, 3); 120];
    small_loss.extend(vec![task(3, 3, 3); 880]);
    let loss = judge(&small_loss, &bar).unwrap().accuracy;
    assert!(loss.intervals.pass().lower > bar.margin);
    assert!(loss.intervals.fail().upper < 0.0);
    assert_eq!(loss.verdict, Decision::Fail);

    let mixed = [task(3, 0, 3), task(0, 3, 3), task(3, 3, 3)];
    assert_eq!(
        judge(&mixed, &bar).unwrap().accuracy.verdict,
        Decision::Inconclusive
    );
}

#[test]
fn void_eval_tasks_leave_both_arms_and_more_than_a_tenth_forces_inconclusive() {
    let bar = Bar::default();
    let mut gain = task(0, 3, 3);
    for score in &mut gain.baseline {
        score.wrong_finding = true;
    }
    let retained = vec![gain; 9];
    let reference = judge(&retained, &bar).unwrap();
    let mut scores = retained;
    let mut dropped = task(3, 0, 3);
    dropped.dropped_as_void = true;
    scores.push(dropped.clone());
    let one_tenth = judge(&scores, &bar).unwrap();
    assert_eq!(one_tenth.accuracy, reference.accuracy);
    assert_eq!(one_tenth.wrong_findings, reference.wrong_findings);
    assert_eq!(one_tenth.retained_tasks, 9);
    assert_eq!(one_tenth.dropped_tasks, 1);
    assert_eq!(one_tenth.verdict, Decision::Pass);

    scores.push(dropped);
    let too_many = judge(&scores, &bar).unwrap();
    assert_eq!(too_many.accuracy, reference.accuracy);
    assert_eq!(too_many.verdict, Decision::Inconclusive);
}

#[test]
fn wrong_findings_reverse_the_sign_and_can_fail_an_accurate_candidate() {
    let bar = Bar::default();
    let mut worse = task(3, 3, 3);
    for score in &mut worse.candidate {
        score.wrong_finding = true;
    }
    let verdict = judge(&vec![worse.clone(); 40], &bar).unwrap();
    assert_eq!(verdict.accuracy.verdict, Decision::Pass);
    assert_eq!(verdict.wrong_findings.verdict, Decision::Fail);
    assert!(verdict.wrong_findings.intervals.fail().upper < 0.0);
    assert_eq!(verdict.verdict, Decision::Fail);

    std::mem::swap(&mut worse.baseline, &mut worse.candidate);
    let improved = judge(&vec![worse; 40], &bar).unwrap();
    assert_eq!(improved.wrong_findings.verdict, Decision::Pass);
    assert!(improved.wrong_findings.intervals.pass().lower > 0.0);
    assert_eq!(improved.verdict, Decision::Pass);
}

#[test]
fn pass_cubed_reports_all_three_successes_per_eval_task_and_is_absent_for_one_trial() {
    let mut scores = vec![task(2, 3, 3); 20];
    scores.extend(vec![task(3, 3, 3); 20]);
    let verdict = judge(&scores, &Bar::default()).unwrap();
    let pass_cubed = verdict.pass_cubed.unwrap();
    assert_eq!(pass_cubed.baseline, 0.5);
    assert_eq!(pass_cubed.candidate, 1.0);

    let one_trial = Bar {
        trials_per_task: 1,
        ..Bar::default()
    };
    let verdict = judge(&vec![task(0, 1, 1); 9], &one_trial).unwrap();
    assert_eq!(verdict.pass_cubed, None);
    // Independent Python random.gammavariate oracle: Dirichlet(1/3, 1/3, 9+1/3)
    // gives approximately [0.5602, 0.9995], with 20,000 draws (seed 42).
    assert!((0.54..0.58).contains(&verdict.accuracy.intervals.pass().lower));
    assert!((0.998..1.0).contains(&verdict.accuracy.intervals.pass().upper));
}

#[test]
fn small_gap_predicts_the_first_eval_task_count_with_eighty_percent_assurance() {
    let mut scores = vec![task(3, 3, 3); 8];
    scores.push(task(3, 2, 3));
    let bar = Bar::default();
    let verdict = judge(&scores, &bar).unwrap();
    assert_eq!(verdict.accuracy.verdict, Decision::Inconclusive);
    let forecast = verdict.accuracy.more_tasks.as_ref().unwrap();
    let additional = forecast.additional_tasks.unwrap();
    assert!(additional > 0 && additional <= bar.prediction.max_additional_tasks);
    // Independent posterior-predictive simulation (Python, seed 42) gave
    // P(clear) = 0.445 at 10 more tasks and 0.92 at 40 more tasks.
    assert!((10..=40).contains(&additional));
    assert_eq!(forecast.target_probability, 0.8);
    assert_eq!(forecast.clear_probabilities.len(), additional);
    assert!(
        forecast.clear_probabilities[..additional - 1]
            .iter()
            .all(|&p| p < 0.8)
    );
    assert!(*forecast.clear_probabilities.last().unwrap() >= 0.8);
    assert_eq!(verdict, judge(&scores, &bar).unwrap());
}

#[test]
fn json_preserves_the_bar_intervals_counts_and_implementation() {
    let bar = Bar {
        seed: 71,
        levels: super::Levels::Separate {
            pass_level: 0.8,
            fail_level: 0.9,
        },
        prior: 1.5,
        ..Bar::default()
    };
    let verdict = judge(&vec![task(3, 3, 3); 40], &bar).unwrap();
    let json = serde_json::to_value(&verdict).unwrap();
    assert_eq!(json["bar"], serde_json::to_value(&bar).unwrap());
    assert_eq!(
        json["accuracy"]["difference_counts"],
        serde_json::json!([0, 0, 0, 40, 0, 0, 0])
    );
    assert_eq!(json["thirdshift_version"], env!("CARGO_PKG_VERSION"));
    assert!(json["method"].as_str().unwrap().contains("Pcg64"));
    assert!(
        json["method"]
            .as_str()
            .unwrap()
            .starts_with("paired-dirichlet-v2;")
    );
    assert_eq!(json["bar"]["pass_level"], 0.8);
    assert_eq!(json["bar"]["fail_level"], 0.9);
    for metric in ["accuracy", "wrong_findings"] {
        let pass = &json[metric]["pass_interval"];
        let fail = &json[metric]["fail_interval"];
        assert!(fail["lower"].as_f64().unwrap() < pass["lower"].as_f64().unwrap());
        assert!(fail["upper"].as_f64().unwrap() > pass["upper"].as_f64().unwrap());
    }
    assert_eq!(json["verdict"], "pass");
    let decoded: Verdict = serde_json::from_str(&serde_json::to_string(&verdict).unwrap()).unwrap();
    assert_eq!(verdict, decoded);
}

#[test]
fn an_exhausted_prediction_budget_is_explicit() {
    let mut bar = Bar::default();
    bar.prediction.max_additional_tasks = 1;
    let scores = [task(3, 0, 3), task(0, 3, 3), task(3, 3, 3)];
    let verdict = judge(&scores, &bar).unwrap();
    let forecast = verdict.accuracy.more_tasks.unwrap();
    assert_eq!(forecast.additional_tasks, None);
    assert_eq!(forecast.clear_probabilities.len(), 1);
    assert!(forecast.clear_probabilities[0] < 0.8);
}

#[test]
fn invalid_bars_and_incomplete_retained_pairs_are_rejected() {
    let mut bar = Bar::default();
    bar.prediction.max_additional_tasks = 1;
    assert!(judge(&[task(1, 1, 1)], &bar).is_err());
    assert!(judge(&[], &bar).is_err());
    let scores = [task(3, 3, 3)];
    for invalid in [0, 2, 4] {
        assert!(
            judge(
                &scores,
                &Bar {
                    trials_per_task: invalid,
                    ..bar.clone()
                }
            )
            .is_err()
        );
    }
    for invalid in [0.0, 1.0, f64::NAN, f64::INFINITY] {
        for levels in [
            super::Levels::Separate {
                pass_level: invalid,
                fail_level: 0.95,
            },
            super::Levels::Separate {
                pass_level: 0.95,
                fail_level: invalid,
            },
        ] {
            assert!(
                judge(
                    &scores,
                    &Bar {
                        levels,
                        ..bar.clone()
                    }
                )
                .is_err()
            );
        }
    }
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            judge(
                &scores,
                &Bar {
                    prior: invalid,
                    ..bar.clone()
                }
            )
            .is_err()
        );
    }
    for invalid in [-1.1, 1.1, f64::NAN] {
        assert!(
            judge(
                &scores,
                &Bar {
                    margin: invalid,
                    ..bar.clone()
                }
            )
            .is_err()
        );
    }
    for invalid in [0, 1] {
        assert!(
            judge(
                &scores,
                &Bar {
                    draws: invalid,
                    ..bar.clone()
                }
            )
            .is_err()
        );
    }
    bar.prediction.simulations = 0;
    assert!(judge(&scores, &bar).is_err());
    bar.prediction.simulations = 20;
    bar.prediction.interval_draws = 1;
    assert!(judge(&scores, &bar).is_err());
    bar.prediction.interval_draws = 100;
    bar.prediction.max_additional_tasks = 0;
    assert!(judge(&scores, &bar).is_err());
}

#[test]
fn an_all_void_suite_keeps_prior_uncertainty_and_an_inconclusive_verdict() {
    let mut bar = Bar::default();
    bar.prediction.max_additional_tasks = 1;
    let scores = [TaskScores {
        baseline: vec![],
        candidate: vec![],
        dropped_as_void: true,
    }];
    let verdict = judge(&scores, &bar).unwrap();
    assert_eq!(verdict.verdict, Decision::Inconclusive);
    assert_eq!(verdict.retained_tasks, 0);
    assert_eq!(verdict.dropped_tasks, 1);
    assert_eq!(verdict.pass_cubed, None);
    assert!(verdict.accuracy.intervals.pass().lower < 0.0);
    assert!(verdict.accuracy.intervals.pass().upper > 0.0);
}
