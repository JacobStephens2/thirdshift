use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::super::grading::{Score, assess};
use super::super::pairwise::ArmPreference;
use super::super::panel::{Merges, Preference};
use super::super::{Arm, Setup, TrialResult};
use super::*;
use crate::logs::Actor;
use crate::trial::Outcome;

const TASKS: usize = 20;

fn tasks() -> Vec<Task> {
    (0..TASKS)
        .map(|index| {
            serde_json::from_value(json!({
                "suite": "process", "name": format!("task-{index:02}"), "directory": "/unused",
                "manifest": {
                    "format": 1, "state": "approved", "session": "process",
                    "repository": "fixture", "base": "a".repeat(40), "change": "b".repeat(40),
                    "criteria": "implement the Ticket", "inputs": {}, "trial": null,
                    "process_grading": {
                        "test_files": ["test_fix.py"], "fail_to_pass": [["true"]],
                        "pass_to_pass": [["true"]],
                        "new_tests": {"paths": ["test_*.py"], "command": ["true"]}
                    }
                }
            }))
            .unwrap()
        })
        .collect()
}

fn bar(panel: Role, people: Option<PeopleSetting>) -> Bar {
    Bar {
        prior: 0.01,
        draws: 512,
        trials_per_task: [("process".to_owned(), 1)].into(),
        panel: (panel != Role::Off).then(|| super::super::panel::Panel {
            seats: Vec::new(),
            vote: super::super::panel::Vote::Family,
            arm_family_rule: true,
            spending_limit: None,
            role: panel,
            families: 2,
            agreement: 0.8,
        }),
        people,
        ..Bar::default()
    }
}

fn people(
    role: Role,
    readers: &[&str],
    sample: Option<usize>,
    overlap: usize,
) -> Option<PeopleSetting> {
    Some(PeopleSetting {
        role,
        readers: readers.iter().map(|reader| (*reader).to_owned()).collect(),
        sample,
        overlap,
    })
}

fn key(index: usize) -> String {
    trial_results::pair_key("process", &format!("task-{index:02}"), 0)
}

/// Code Grader results: both Arms pass unless `candidate_fails` holds.
fn results(candidate_fails: impl Fn(usize) -> bool) -> Vec<TrialResult> {
    (0..TASKS)
        .flat_map(|index| {
            [Arm::Baseline, Arm::Candidate].map(|arm| TrialResult {
                suite: "process".into(),
                task: format!("task-{index:02}"),
                arm,
                trial: 0,
                outcome: Outcome::Delivered,
                score: Some(Score {
                    passed: arm == Arm::Baseline || !candidate_fails(index),
                    wrong_finding: false,
                    borderline: false,
                    model: None,
                    checks: Vec::new(),
                    rechecks: Vec::new(),
                }),
                attempts: Vec::new(),
                leaks: Vec::new(),
            })
        })
        .collect()
}

/// The panel calls the Baseline ready and the Candidate not ready on every pair.
fn panel_against_candidate() -> Vec<PanelPair> {
    (0..TASKS)
        .map(|index| PanelPair {
            suite: "process".into(),
            task: format!("task-{index:02}"),
            trial: 0,
            graders: Vec::new(),
            votes: Vec::new(),
            preference: Preference::Baseline,
            merge: Merges {
                baseline: Merge::Ready,
                candidate: Merge::NotReady,
            },
        })
        .collect()
}

fn review(pair: &str, reader: &str, baseline_ready: bool, candidate_ready: bool) -> Review {
    let index = (0..TASKS).find(|index| key(*index) == pair).unwrap();
    Review {
        pair: pair.to_owned(),
        suite: "process".into(),
        task: format!("task-{index:02}"),
        trial: 0,
        reader: Actor::console(reader.to_owned(), None),
        recorded_at: "2026-11-10T09:00:00+00:00".into(),
        blind: true,
        baseline_ready,
        candidate_ready,
        preference: Some(ArmPreference::Tie),
        rubric: BTreeMap::new(),
    }
}

/// Every reader's blind calls on every pair in their queue.
fn read_queues(bar: &Bar, baseline_ready: bool, candidate_ready: bool) -> Vec<Review> {
    queues(bar, &tasks())
        .iter()
        .flat_map(|queue| {
            queue
                .pairs
                .iter()
                .map(|pair| review(pair, &queue.reader, baseline_ready, candidate_ready))
        })
        .collect()
}

fn judged(bar: &Bar, results: &[TrialResult], reviews: &[Review]) -> Value {
    let tasks = tasks();
    let decided = decide(bar, &tasks, &panel_against_candidate(), reviews).unwrap();
    let setup = Setup {
        commit: Some("a".repeat(40)),
        harness: Some("claude".into()),
        model: None,
        effort: None,
        process: None,
        process_digest: None,
        runner: None,
        runner_digest: None,
    };
    serde_json::to_value(assess(&tasks, results, bar, &setup, &decided).unwrap().0).unwrap()
}

#[test]
fn the_counted_panel_decides_every_pair_while_people_calibrate_beside() {
    let bar = bar(Role::Counted, people(Role::Beside, &["ann"], Some(4), 0));
    // People call both ready; beside the Verdict, they change nothing.
    let verdict = judged(&bar, &results(|_| false), &read_queues(&bar, true, true));
    assert_eq!(verdict["verdict"], "fail", "{verdict}");
    assert_eq!(
        verdict["deciders"],
        json!({"panel": "counted", "people": "beside", "people_pairs": 0, "panel_pairs": 20,
               "waiting_pairs": 0, "left_out_pairs": 0}),
    );
}

#[test]
fn people_counted_on_a_sample_decide_on_the_sampled_pairs_alone() {
    let bar = bar(Role::Beside, people(Role::Counted, &["ann"], Some(12), 0));
    let sampled: BTreeSet<String> = queues(&bar, &tasks())[0].pairs.iter().cloned().collect();
    assert_eq!(sampled.len(), 12);
    // The code Grader fails the Candidate on every unsampled pair, and the
    // panel beside the Verdict calls it not ready everywhere: neither counts.
    let results = results(|index| !sampled.contains(&key(index)));
    let verdict = judged(&bar, &results, &read_queues(&bar, true, true));
    // Every Score is decisive here, so the pass waits on rechecks.
    assert_eq!(verdict["provisional_verdict"], "pass", "{verdict}");
    let suite = &verdict["suites"]["process"];
    assert_eq!(suite["retained_tasks"], 12, "each sampled pair is one unit");
    assert_eq!(suite["bar"]["trials_per_task"], 1);
    assert_eq!(verdict["deciders"]["people_pairs"], 12);
    assert_eq!(verdict["deciders"]["left_out_pairs"], 8);
    assert_eq!(verdict["deciders"]["panel_pairs"], 0);
    // A sampled pair the reader calls not ready fails that Trial.
    let mut reviews = read_queues(&bar, true, true);
    for review in &mut reviews {
        review.candidate_ready = false;
    }
    assert_eq!(
        judged(&bar, &results, &reviews)["provisional_verdict"],
        "fail"
    );
}

fn verdict_of(bar: &Bar, results: &[TrialResult], reviews: &[Review]) -> Value {
    judged(bar, results, reviews)["verdict"].clone()
}

#[test]
fn with_both_counted_the_panel_stands_in_on_pairs_people_did_not_read() {
    let bar = bar(Role::Counted, people(Role::Counted, &["ann"], Some(4), 0));
    // People agree with the panel on their four pairs: Baseline ready,
    // Candidate not. The panel's calls decide the other sixteen.
    let verdict = judged(&bar, &results(|_| false), &read_queues(&bar, true, false));
    assert_eq!(verdict["verdict"], "fail", "{verdict}");
    assert_eq!(
        verdict["deciders"],
        json!({"panel": "counted", "people": "counted", "people_pairs": 4, "panel_pairs": 16,
               "waiting_pairs": 0, "left_out_pairs": 0,
               "sample_agreement": {"compared": 8, "agreed": 8}}),
    );
}

#[test]
fn a_stand_in_with_low_people_panel_agreement_is_inconclusive_and_says_why() {
    let bar = bar(Role::Counted, people(Role::Counted, &["ann"], Some(4), 0));
    // People call the Candidate ready where the panel does not: 4 of 8.
    let verdict = judged(&bar, &results(|_| false), &read_queues(&bar, true, true));
    assert_eq!(verdict["verdict"], "inconclusive", "{verdict}");
    assert_eq!(
        verdict["deciders"]["sample_agreement"],
        json!({"compared": 8, "agreed": 4})
    );
    let reason = verdict["deciders"]["inconclusive"].as_str().unwrap();
    assert!(
        reason.contains("4/8 merge calls, below the Bar's 0.8"),
        "{reason}"
    );
    let line = verdict["line"].as_str().unwrap();
    assert!(
        line.starts_with("inconclusive (people–panel agreement"),
        "{line}"
    );
}

#[test]
fn people_counted_on_every_pair_decide_every_pair() {
    let bar = bar(Role::Beside, people(Role::Counted, &["ann"], None, 0));
    assert_eq!(queues(&bar, &tasks())[0].pairs.len(), TASKS);
    // The panel beside calls the Candidate not ready; people call it ready.
    let verdict = judged(&bar, &results(|_| false), &read_queues(&bar, true, true));
    assert_eq!(verdict["provisional_verdict"], "pass", "{verdict}");
    assert_eq!(verdict["deciders"]["people_pairs"], 20);
    assert_eq!(verdict["deciders"]["left_out_pairs"], 0);
    assert_eq!(verdict["suites"]["process"]["bar"]["trials_per_task"], 1);
    assert_eq!(verdict["suites"]["process"]["retained_tasks"], 20);
}

#[test]
fn counted_people_hold_the_verdict_until_every_reader_has_called() {
    let bar = bar(Role::Beside, people(Role::Counted, &["ann"], Some(3), 0));
    let mut reviews = read_queues(&bar, true, true);
    reviews.pop();
    let verdict = judged(&bar, &results(|_| false), &reviews);
    assert_eq!(verdict["verdict"], "waiting_on_people", "{verdict}");
    assert_eq!(verdict["deciders"]["waiting_pairs"], 1);
    // A review begun after the reader's reveal is not a blind call.
    let mut late = reviews;
    let missing = queues(&bar, &tasks())[0].pairs[2].clone();
    let mut seen = review(&missing, "ann", true, true);
    seen.blind = false;
    late.push(seen);
    assert_eq!(
        verdict_of(&bar, &results(|_| false), &late),
        "waiting_on_people"
    );
}

#[test]
fn two_readers_share_exactly_the_overlap_and_a_split_call_is_not_ready() {
    let bar = bar(Role::Off, people(Role::Counted, &["ann", "bo"], Some(5), 2));
    let drawn = queues(&bar, &tasks());
    let [ann, bo] = [&drawn[0], &drawn[1]].map(|queue| {
        assert_eq!(queue.pairs.len(), 5, "{}", queue.reader);
        queue.pairs.iter().cloned().collect::<BTreeSet<_>>()
    });
    let shared: Vec<_> = ann.intersection(&bo).cloned().collect();
    assert_eq!(shared.len(), 2);
    // The same seed draws the same queues; another seed draws others.
    let again = queues(&bar, &tasks());
    assert_eq!(again[0].pairs, drawn[0].pairs);
    assert_eq!(again[1].pairs, drawn[1].pairs);
    let reseeded = Bar {
        seed: 7,
        ..bar.clone()
    };
    assert!((0..8).any(|seed| {
        queues(
            &Bar {
                seed,
                ..reseeded.clone()
            },
            &tasks(),
        )[0]
        .pairs
            != drawn[0].pairs
    }));

    let mut reviews = read_queues(&bar, true, true);
    reviews.retain(|review| !(review.pair == shared[0] && review.reader.who() == "bo"));
    reviews.push(review(&shared[0], "bo", true, false));
    let decided = decide(&bar, &tasks(), &[], &reviews).unwrap();
    assert_eq!(
        decided.calls[&shared[0]],
        Call::Ready {
            baseline: true,
            candidate: false,
            people: true
        },
        "a disagreement on a shared pair counts as not ready"
    );
    assert_eq!(
        decided.calls[&shared[1]],
        Call::Ready {
            baseline: true,
            candidate: true,
            people: true
        }
    );
}

#[test]
fn a_sample_the_experiment_cannot_hold_is_refused() {
    let tasks = tasks();
    let fits = bar(
        Role::Off,
        people(Role::Counted, &["ann", "bo"], Some(11), 2),
    );
    assert!(check(&fits, &tasks).is_ok());
    let too_big = bar(
        Role::Off,
        people(Role::Counted, &["ann", "bo"], Some(12), 2),
    );
    let error = check(&too_big, &tasks).unwrap_err().to_string();
    assert!(
        error.contains("need 22 pairs (12 per reader, 2 shared), but the Experiment has 20"),
        "{error}"
    );
}

#[test]
fn the_people_flags_keep_the_old_setting_unless_named() {
    let parse = |args: &[(&'static str, &'static str)], readers: &[&str]| {
        let mut flags: BTreeMap<&str, &str> = args.iter().copied().collect();
        PeopleSetting::parse(
            &mut flags,
            readers.iter().map(|reader| (*reader).to_owned()).collect(),
        )
    };
    assert_eq!(parse(&[], &[]).unwrap(), None);
    assert_eq!(parse(&[("--people", "beside")], &[]).unwrap(), None);
    assert_eq!(
        parse(
            &[
                ("--people", "counted"),
                ("--people-sample", "4"),
                ("--people-overlap", "1")
            ],
            &["ann", "bo"]
        )
        .unwrap(),
        people(Role::Counted, &["ann", "bo"], Some(4), 1)
    );
    for (args, readers, error) in [
        (
            &[("--people", "counted")][..],
            &[][..],
            "--people counted needs at least one --people-reader",
        ),
        (
            &[("--people-sample", "3"), ("--people-overlap", "4")],
            &["ann"],
            "--people-overlap cannot exceed --people-sample",
        ),
        (
            &[("--people", "off")],
            &["ann"],
            "--people off takes no --people-reader, --people-sample or --people-overlap",
        ),
        (
            &[("--people", "maybe")],
            &[],
            "invalid --people maybe: expected off, beside or counted",
        ),
    ] {
        assert_eq!(parse(args, readers).unwrap_err().to_string(), error);
    }
}

#[test]
fn a_sample_only_verdict_leaves_other_suites_statistics_alone() {
    let mut tasks = tasks();
    for index in 0..4 {
        tasks.push(
            serde_json::from_value(json!({
                "suite": "review", "name": format!("review-{index}"), "directory": "/unused",
                "manifest": {"format": 1, "state": "approved", "session": "code-review",
                    "repository": "fixture", "base": "a".repeat(40), "change": "b".repeat(40),
                    "criteria": "find the bug", "inputs": {}, "trial": null}
            }))
            .unwrap(),
        );
    }
    let mut bar = bar(Role::Beside, people(Role::Counted, &["ann"], Some(12), 0));
    bar.trials_per_task.insert("review".into(), 3);
    let mut results = results(|_| false);
    for index in 0..4 {
        for trial in 0..3 {
            for arm in [Arm::Baseline, Arm::Candidate] {
                results.push(TrialResult {
                    suite: "review".into(),
                    task: format!("review-{index}"),
                    arm,
                    trial,
                    outcome: Outcome::Delivered,
                    score: Some(Score {
                        passed: true,
                        wrong_finding: false,
                        borderline: false,
                        model: None,
                        checks: Vec::new(),
                        rechecks: Vec::new(),
                    }),
                    attempts: Vec::new(),
                    leaks: Vec::new(),
                });
            }
        }
    }
    let decided = decide(
        &bar,
        &tasks,
        &panel_against_candidate(),
        &read_queues(&bar, true, true),
    )
    .unwrap();
    let setup = Setup {
        commit: Some("a".repeat(40)),
        harness: Some("claude".into()),
        model: None,
        effort: None,
        process: None,
        process_digest: None,
        runner: None,
        runner_digest: None,
    };
    let verdict =
        serde_json::to_value(assess(&tasks, &results, &bar, &setup, &decided).unwrap().0).unwrap();
    assert_eq!(
        verdict["suites"]["review"]["bar"]["trials_per_task"], 3,
        "{verdict}"
    );
    assert_eq!(
        verdict["suites"]["review"]["retained_tasks"], 4,
        "{verdict}"
    );
}
