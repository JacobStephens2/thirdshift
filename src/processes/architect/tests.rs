use super::*;
use crate::github;
use crate::pass::{Call, InMemory, ready_for_review, widgets};
use std::path::Path;

const ORIGIN: &str = "git@github.com:acme/widgets.git";

fn started() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-04T09:00:00.750Z")
        .unwrap()
        .to_utc()
}

fn viewed(open: bool, after: i64, labels: &[&str]) -> github::Issue {
    github::Issue {
        title: None,
        is_open: open,
        labels: labels.iter().copied().collect(),
        created: started() + chrono::Duration::seconds(after),
    }
}

fn url(number: u64) -> String {
    format!("https://github.com/acme/widgets/issues/{number}")
}

fn message(line: &str) -> String {
    format!("Reviewed the codebase.\n\n{line}{}\n", url(8))
}

fn step(line: &str) -> Call {
    Call::Step(line.to_string())
}

/// An Architect run on `repo`, past its lock and gates, with no focus, no pull,
/// and no plan-only, on the Base branch `main`, except as `asked`
/// changes them, its dispatched run ending ready for review: how it
/// ended, and what it did outside itself.
fn architect(repo: InMemory, asked: impl FnOnce(&mut ArchitectRun<'_>)) -> (Outcome, Vec<Call>) {
    let mut repo = repo.dispatched_ending(ready_for_review());
    let widgets = widgets();
    let mut architect_run = ArchitectRun {
        repo: &widgets,
        base: "main",
        origin: ORIGIN,
        focus: None,
        started: started(),
        pull: false,
        plan_only: false,
    };
    asked(&mut architect_run);
    let outcome = run(&mut repo, &architect_run);
    (outcome, repo.calls)
}

/// What an Architect run that got past its gates came to: how its review
/// ended, as its line or its cause, and whether it dispatched a run.
fn review_and_dispatch(outcome: Outcome) -> (Result<String, String>, bool) {
    let Outcome::Ran { review, dispatched } = outcome else {
        panic!("the Architect run was skipped");
    };
    let review = review
        .map(|reviewed| reviewed.to_string())
        .map_err(|failed| format!("{:#}", failed.error));
    (review, dispatched.is_some())
}

/// The calls that start the Architecture review of `main`, with no
/// focus.
fn reviewing_main() -> [Call; 2] {
    [
        Call::Review {
            base: "main".to_string(),
            prompt: prompt::architecture_review("main", None),
        },
        step("starting the Architecture review of main"),
    ]
}

/// A repository whose review publishes plan #8, which passes its checks.
fn publishing_a_plan() -> InMemory {
    InMemory::default()
        .reviewed(&message(prompt::PLAN_LINE))
        .viewable(8, viewed(true, 0, &["needs-triage"]))
}

#[test]
fn a_harness_that_fails_its_check_fails_the_architect_run_before_its_start() {
    let repo = publishing_a_plan().harness_failing();

    let (outcome, calls) = architect(repo, |run| run.pull = true);

    assert_eq!(
        review_and_dispatch(outcome),
        (Err("claude is not on PATH".to_string()), false)
    );
    assert_eq!(calls, [Call::HarnessCheck]);
}

#[test]
fn the_pull_is_made_only_when_asked_after_the_start_is_recorded() {
    for pull in [false, true] {
        let (_, calls) = architect(publishing_a_plan(), |run| run.pull = pull);

        let mut expected = vec![Call::HarnessCheck, Call::Started(None)];
        if pull {
            expected.push(Call::Pull);
        }
        expected.extend(reviewing_main());
        assert_eq!(calls[..expected.len()], expected, "pull: {pull}");
    }
}

#[test]
fn an_interrupt_after_the_gates_fails_the_architect_run_as_interrupted_with_no_review() {
    let repo = publishing_a_plan().interrupted();

    let (outcome, calls) = architect(repo, |run| run.pull = true);

    let Outcome::Ran {
        review: Err(failed),
        dispatched: None,
    } = outcome
    else {
        panic!("the Architect run did not fail, or dispatched a run");
    };
    assert_eq!(failed.error.to_string(), "interrupted");
    assert!(failed.interrupted);
    assert_eq!(failed.log, None);
    assert_eq!(
        calls,
        [Call::HarnessCheck, Call::Started(None), Call::Pull,]
    );
}

#[test]
fn the_reviews_prompt_carries_the_base_branch_and_the_focus() {
    let (_, calls) = architect(publishing_a_plan(), |run| {
        run.base = "develop";
        run.focus = Some("the Spec run");
    });

    let review = calls
        .iter()
        .position(|call| matches!(call, Call::Review { .. }))
        .expect("no review session");
    let Call::Review { base, prompt } = &calls[review] else {
        unreachable!();
    };
    assert_eq!(
        calls[review + 1],
        step("starting the Architecture review of develop, focused on: the Spec run")
    );
    assert_eq!(base, "develop");
    assert!(
        prompt.contains("the base branch develop:"),
        "prompt: {prompt}"
    );
    assert!(
        prompt.contains("Focus the review on: the Spec run\n"),
        "prompt: {prompt}"
    );
}

#[test]
fn a_plan_is_dispatched_on_the_base_branch_after_the_label_swap_unless_plan_only() {
    for plan_only in [false, true] {
        let (outcome, calls) = architect(publishing_a_plan(), |run| {
            run.base = "develop";
            run.plan_only = plan_only;
        });

        assert_eq!(
            review_and_dispatch(outcome),
            (
                Ok(format!("plan {} is ready for an agent", url(8))),
                !plan_only
            )
        );
        let swap = calls
            .iter()
            .position(|call| matches!(call, Call::Edit { .. }))
            .expect("the plan was not marked ready");
        let dispatch = [
            step(&format!(
                "dispatching the plan {url}, as thirdshift {url} would",
                url = url(8)
            )),
            Call::DispatchPlan {
                plan: 8,
                base: "develop".to_string(),
            },
        ];
        let after_swap: &[Call] = if plan_only { &[] } else { &dispatch };
        assert_eq!(calls[swap + 1..], *after_swap, "plan-only: {plan_only}");
    }
}

#[test]
fn an_idea_filed_or_already_filed_dispatches_nothing() {
    for line in [prompt::IDEA_LINE, prompt::ALREADY_FILED_LINE] {
        let repo = InMemory::default()
            .reviewed(&message(line))
            .viewable(8, viewed(true, -3600, &[]));

        let (outcome, calls) = architect(repo, |_| {});

        let (review, dispatched) = review_and_dispatch(outcome);
        assert!(review.is_ok(), "{line}: {review:?}");
        assert!(!dispatched, "{line}");
        assert!(
            matches!(calls.last(), Some(Call::Edit { .. })),
            "{line}: {calls:?}"
        );
    }
}

const SESSION_LOG: &str = "/logs/acme/widgets/sessions/architect-1-architecture-review.jsonl";

#[test]
fn a_failed_review_dispatches_nothing_and_carries_the_reviews_session_log() {
    for repo in [
        InMemory::default().review_failing("claude exited with status 1"),
        InMemory::default().reviewed("Published the plan."),
        InMemory::default()
            .reviewed(&message(prompt::PLAN_LINE))
            .viewable(8, viewed(false, 0, &["needs-triage"])),
    ] {
        let (outcome, calls) = architect(repo.session_log(SESSION_LOG), |_| {});

        let Outcome::Ran {
            review: Err(failed),
            dispatched: None,
        } = outcome
        else {
            panic!("the review did not fail, or a run was dispatched");
        };
        assert_eq!(
            failed.log.as_deref(),
            Some(Path::new(SESSION_LOG)),
            "{:#}",
            failed.error
        );
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, Call::DispatchPlan { .. })),
            "{:#}",
            failed.error
        );
    }
}
