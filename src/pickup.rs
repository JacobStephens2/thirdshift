//! A Pickup run: from the Launch directory with no Issue URL, the search
//! for the lowest-numbered Ready issue in the repository, saying why it
//! passed over each issue labelled `ready-for-agent` before it, then that
//! issue dispatched as a Spec run or a Run. Only one Pass per repository
//! runs at a time on a machine: one started while another Pass is still
//! running is skipped, before any search. One that runs first makes
//! the Sweep, and is then skipped if it finds the repository at its Claim
//! limit, or with no Ready issue.

use std::fmt;
use std::num::NonZeroUsize;

use anyhow::Result;

use crate::asks::Flags;
use crate::claim;
use crate::config::UserConfig;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::Edit;
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{Dispatch, LaunchAndGitHub, Outside};
use crate::ready::ReadyIssue;
use crate::run::Ended;

/// How a Pickup run ended, short of a failure.
pub enum Outcome {
    /// It took a Ready issue and dispatched it.
    Took(Took),
    /// Skipped, having done nothing.
    Skipped(Skipped),
}

/// The Ready issue a Pickup run took, and how the run it dispatched ended.
pub struct Took {
    pub issue: IssueUrl,
    /// Its title, as the search listed it.
    pub title: String,
    /// How the Spec run or Run it was dispatched as ended.
    pub ended: Ended,
}

/// Why a Pickup run was skipped. Its `Display` is the reason, as the skipped
/// run's one line gives it.
pub enum Skipped {
    /// Another Pass on this repository is still
    /// running on this machine, the Spec run or Run it dispatched included.
    AlreadyRunning(AlreadyRunning),
    /// The repository is at its Claim limit: `claimed` open issues carry a
    /// Claim, and `limit` or more stop a Pickup run from taking another.
    AtClaimLimit {
        repo: Repo,
        claimed: usize,
        limit: NonZeroUsize,
    },
    /// The repository has no Ready issue.
    NoReadyIssue(Repo),
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(running) => running.fmt(f),
            Self::AtClaimLimit {
                repo,
                claimed,
                limit,
            } => write!(
                f,
                "at the Claim limit on {}: {claimed} open issue(s) labelled {}, \
                 pickup.limit is {limit}",
                repo.slug(),
                claim::IN_PROGRESS
            ),
            Self::NoReadyIssue(repo) => write!(f, "no Ready issue on {}", repo.slug()),
        }
    }
}

/// Take the lowest-numbered Ready issue in the Launch directory's
/// repository, saying which on stderr, after a line there on each issue
/// labelled `ready-for-agent` it passed over on the way, with why, and
/// dispatch it as `thirdshift <Issue URL>` with `flags` would run it, but on
/// the Pickup run's Base branch. The Base branch is `base`, the branch the
/// command named, whatever the Launch directory has checked out, or without
/// one the branch checked out there. Nothing is changed in the Launch
/// directory, nor on the issue taken: the run it is dispatched as makes the
/// Claim.
///
/// Once the preflight checks pass, and before any search, the Pickup run is
/// skipped if another Pass on the same repository
/// is still running on this machine. Otherwise this process is that
/// repository's one such run until it exits, through whatever it dispatches.
/// A skip is recorded in the repository's Activity log. The rest, from the
/// Sweep to the dispatch, is [`run_through`], with `config`'s Claim limit,
/// through the Launch directory, GitHub, the logs and `harness`, the
/// Harness, Model and Effort the run it dispatches will run its sessions on,
/// and `flags` and `config`, which ask that run.
pub fn run(
    base: Option<&str>,
    flags: &Flags,
    config: &UserConfig,
    harness: &mut Choice,
) -> Result<Outcome> {
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base)? {
        Start::Clear(launch) => launch,
        Start::AlreadyRunning(running) => {
            logs::skipped(Pass::PickupRun, &running.0, &running);
            return Ok(Outcome::Skipped(Skipped::AlreadyRunning(running)));
        }
    };
    let mut outside = LaunchAndGitHub {
        launch: &directory,
        base: &base,
        repo: &repo,
        harness,
        flags,
        config,
    };
    run_through(&mut outside, &repo, base.name(), config.pickup_limit)
}

/// [`run`], once it holds its repository's lock, on `repo` with the Base
/// branch `base` and the Claim limit `limit`, through `outside`: its
/// [`gates`], a skip recorded with its reason if one skips; then the line
/// naming the issue taken; then the check of its Harness, failing before
/// any work if it can't run; then the record that it started work on the
/// issue, which keeps its Command log; then the issue dispatched, a Spec
/// run on a Spec and a Run otherwise, on `base`.
fn run_through(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    limit: NonZeroUsize,
) -> Result<Outcome> {
    let ReadyIssue { listed, is_spec } = match gates(outside, repo, limit)? {
        Decision::Take(ready) => ready,
        Decision::Skip(skipped) => {
            outside.skipped(Pass::PickupRun, &skipped);
            return Ok(Outcome::Skipped(skipped));
        }
    };
    outside.step(format!(
        "taking Ready issue #{} \"{}\", as thirdshift {} would",
        listed.issue.number, listed.title, listed.issue.url
    ));
    outside.check_harness()?;
    outside.started(Work::PickupRun(&listed.issue));
    let ended = outside.dispatch(Dispatch::ReadyIssue {
        issue: &listed.issue,
        is_spec,
        base,
    });
    Ok(Outcome::Took(Took {
        issue: listed.issue,
        title: listed.title,
        ended,
    }))
}

/// What a Pickup run's gates decided.
enum Decision {
    /// It is skipped, for this reason.
    Skip(Skipped),
    /// It takes this Ready issue.
    Take(ReadyIssue),
}

/// The gates of a Pickup run on `repo`, reaching it through `outside`: the
/// Sweep, then, if `limit` or more of its open issues carry a Claim, a skip
/// for the Claim limit, then the Ready issue search, a skip if it finds
/// none. No gate is read once one skips.
fn gates(outside: &mut impl Outside, repo: &Repo, limit: NonZeroUsize) -> Result<Decision> {
    sweep(outside);
    let claimed = outside.open_issues(claim::IN_PROGRESS)?.len();
    if claimed >= limit.get() {
        return Ok(Decision::Skip(Skipped::AtClaimLimit {
            repo: repo.clone(),
            claimed,
            limit,
        }));
    }
    Ok(match outside.ready_issue()? {
        Some(ready) => Decision::Take(ready),
        None => Decision::Skip(Skipped::NoReadyIssue(repo.clone())),
    })
}

/// The Sweep: take `in-progress` off every closed issue that still carries
/// it, as an issue merged by hand does, leaving its other labels. A
/// failure, to list them or to take the label off one, is only a warning.
fn sweep(outside: &mut impl Outside) {
    let closed = match outside.closed_issues(claim::IN_PROGRESS) {
        Ok(closed) => closed,
        Err(error) => {
            outside.step(format!(
                "warning: could not list the closed issues labelled {}: {error:#}",
                claim::IN_PROGRESS
            ));
            return;
        }
    };
    for ListedIssue { issue, labels, .. } in closed {
        let edit = Edit::of(&issue, labels, &[claim::IN_PROGRESS], &[]);
        if !edit.takes_off(claim::IN_PROGRESS) {
            continue;
        }
        outside.step(format!(
            "taking {} off #{}, which is closed",
            claim::IN_PROGRESS,
            issue.number
        ));
        if let Err(error) = outside.apply(&edit) {
            outside.step(format!(
                "warning: could not take {} off #{}: {error:#}",
                claim::IN_PROGRESS,
                issue.number
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, PR_URL, ready_for_review, widgets};

    /// The gates of a Pickup run on `repo` with the Claim limit `limit`:
    /// the line of the reason it is skipped, or the number of the Ready
    /// issue it takes and whether it is a Spec, and what the gates did.
    fn decide(mut repo: InMemory, limit: usize) -> (Result<(u64, bool), String>, Vec<Call>) {
        let limit = NonZeroUsize::new(limit).unwrap();
        let decided = match gates(&mut repo, &widgets(), limit).unwrap() {
            Decision::Take(ready) => Ok((ready.listed.issue.number, ready.is_spec)),
            Decision::Skip(skipped) => Err(skipped.to_string()),
        };
        (decided, repo.calls)
    }

    const CLOSED: Call = Call::Closed("in-progress");
    const CLAIMED: Call = Call::Open("in-progress");

    /// The Sweep taking `in-progress` off issue `number`, leaving `labels`.
    fn swept(number: u64, labels: &[&str]) -> [Call; 2] {
        [
            Call::Step(format!("taking in-progress off #{number}, which is closed")),
            Call::Edit {
                issue: number,
                off: vec!["in-progress"],
                on: vec![],
                labels: labels.iter().map(|label| label.to_string()).collect(),
            },
        ]
    }

    const NO_READY_ISSUE: &str = "no Ready issue on acme/widgets";

    #[test]
    fn the_ready_issue_is_taken_with_its_spec_or_not_answer() {
        for is_spec in [false, true] {
            let repo = InMemory::default().ready(9, is_spec);

            let (decided, calls) = decide(repo, 3);

            assert_eq!(decided, Ok((9, is_spec)));
            assert_eq!(calls, [CLOSED, CLAIMED, Call::ReadySearch]);
        }
    }

    #[test]
    fn with_no_ready_issue_the_pass_is_skipped() {
        let (decided, _) = decide(InMemory::default(), 3);

        assert_eq!(decided, Err(NO_READY_ISSUE.to_string()));
    }

    #[test]
    fn the_sweep_takes_in_progress_off_each_closed_issue_leaving_its_other_labels() {
        let repo = InMemory::default()
            .issue(4, false, &["bug", "in-progress", "p1"])
            .issue(5, false, &["In-Progress"])
            .issue(6, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = vec![CLOSED];
        expected.extend(swept(4, &["bug", "p1"]));
        expected.extend(swept(5, &[]));
        expected.extend([CLAIMED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_skips_a_closed_issue_its_edit_would_take_nothing_off() {
        let repo = InMemory::default()
            .filed("in-progress", 4, false, &["bug"])
            .issue(5, false, &["in-progress"])
            .ready(9, false);

        let (_, calls) = decide(repo, 3);

        let mut expected = vec![CLOSED];
        expected.extend(swept(5, &[]));
        expected.extend([CLAIMED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn a_sweep_that_cannot_list_the_closed_issues_warns_and_the_pass_carries_on() {
        let repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .ready(9, false)
            .closed_listing_failing();

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        assert_eq!(
            calls,
            [
                CLOSED,
                Call::Step(
                    "warning: could not list the closed issues labelled in-progress: \
                     gh: could not list the closed issues"
                        .to_string()
                ),
                CLAIMED,
                Call::ReadySearch,
            ]
        );
    }

    #[test]
    fn a_sweep_that_cannot_take_in_progress_off_one_issue_warns_and_carries_on() {
        let repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .issue(5, false, &["in-progress"])
            .ready(9, false)
            .edit_failing(4);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = vec![CLOSED];
        expected.extend(swept(4, &[]));
        expected.push(Call::Step(
            "warning: could not take in-progress off #4: gh: could not edit #4".to_string(),
        ));
        expected.extend(swept(5, &[]));
        expected.extend([CLAIMED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_comes_before_the_count_on_a_pass_skipped_for_the_claim_limit() {
        let repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 1);

        assert!(decided.is_err());
        let mut expected = vec![CLOSED];
        expected.extend(swept(4, &[]));
        expected.push(CLAIMED);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_comes_before_the_count_on_a_pass_skipped_for_no_ready_issue() {
        let repo = InMemory::default().issue(4, false, &["in-progress"]);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Err(NO_READY_ISSUE.to_string()));
        let mut expected = vec![CLOSED];
        expected.extend(swept(4, &[]));
        expected.extend([CLAIMED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn at_exactly_the_claim_limit_the_pass_is_skipped_with_no_ready_issue_search() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["In-Progress", "bug"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 2);

        assert_eq!(
            decided,
            Err(
                "at the Claim limit on acme/widgets: 2 open issue(s) labelled in-progress, \
                 pickup.limit is 2"
                    .to_string()
            )
        );
        assert_eq!(calls, [CLOSED, CLAIMED]);
    }

    #[test]
    fn over_the_claim_limit_the_pass_is_skipped_naming_the_count() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .issue(6, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 2);

        assert_eq!(
            decided,
            Err(
                "at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, \
                 pickup.limit is 2"
                    .to_string()
            )
        );
        assert_eq!(calls, [CLOSED, CLAIMED]);
    }

    #[test]
    fn one_below_the_claim_limit_the_pass_takes_the_ready_issue() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        assert_eq!(calls, [CLOSED, CLAIMED, Call::ReadySearch]);
    }

    /// A Pickup run on `repo`, past its lock, on the Base branch `main` with
    /// the Claim limit 3, its dispatched run ending ready for review: how it
    /// ended, and what it did outside itself.
    fn pickup(repo: InMemory) -> (Result<Outcome>, Vec<Call>) {
        let mut repo = repo.dispatched_ending(ready_for_review());
        let limit = NonZeroUsize::new(3).unwrap();
        let outcome = run_through(&mut repo, &widgets(), "main", limit);
        (outcome, repo.calls)
    }

    /// The line naming Ready issue #9 as taken.
    fn taking_9() -> Call {
        Call::Step(
            "taking Ready issue #9 \"Issue 9\", as thirdshift \
             https://github.com/acme/widgets/issues/9 would"
                .to_string(),
        )
    }

    #[test]
    fn a_pass_skipped_for_no_ready_issue_records_its_reason_and_does_no_work() {
        let (outcome, calls) = pickup(InMemory::default());

        assert!(matches!(
            outcome,
            Ok(Outcome::Skipped(Skipped::NoReadyIssue(_)))
        ));
        assert_eq!(
            calls,
            [
                CLOSED,
                CLAIMED,
                Call::ReadySearch,
                Call::Skipped(NO_READY_ISSUE.to_string()),
            ]
        );
    }

    #[test]
    fn a_pass_skipped_at_the_claim_limit_records_its_reason_and_does_no_work() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .issue(6, true, &["in-progress"])
            .ready(9, false);

        let (outcome, calls) = pickup(repo);

        assert!(matches!(
            outcome,
            Ok(Outcome::Skipped(Skipped::AtClaimLimit { .. }))
        ));
        assert_eq!(
            calls,
            [
                CLOSED,
                CLAIMED,
                Call::Skipped(
                    "at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, \
                     pickup.limit is 3"
                        .to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_pass_that_takes_an_issue_names_it_checks_its_harness_starts_then_dispatches_it() {
        let (_, calls) = pickup(InMemory::default().ready(9, false));

        assert_eq!(
            calls,
            [
                CLOSED,
                CLAIMED,
                Call::ReadySearch,
                taking_9(),
                Call::HarnessCheck,
                Call::Started(Some(9)),
                Call::DispatchReadyIssue {
                    issue: 9,
                    is_spec: false,
                    base: "main".to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_spec_is_dispatched_as_one_on_the_pickup_runs_base_branch() {
        let (_, calls) = pickup(InMemory::default().ready(9, true));

        assert_eq!(
            calls.last(),
            Some(&Call::DispatchReadyIssue {
                issue: 9,
                is_spec: true,
                base: "main".to_string(),
            })
        );
    }

    #[test]
    fn a_harness_that_fails_its_check_fails_the_pass_with_no_start_and_no_dispatch() {
        let repo = InMemory::default().ready(9, false).harness_failing();

        let (outcome, calls) = pickup(repo);

        let error = outcome.err().expect("the pass did not fail");
        assert_eq!(format!("{error:#}"), "claude is not on PATH");
        assert_eq!(&calls[calls.len() - 2..], [taking_9(), Call::HarnessCheck]);
    }

    #[test]
    fn a_pass_that_took_an_issue_returns_it_with_how_its_dispatched_run_ended() {
        let (outcome, _) = pickup(InMemory::default().ready(9, false));

        let Ok(Outcome::Took(took)) = outcome else {
            panic!("the pass took no issue");
        };
        assert_eq!(took.issue.number, 9);
        assert_eq!(took.title, "Issue 9");
        let reached = took.ended.outcome.ok().expect("the dispatched run failed");
        assert_eq!(reached.pr_url, PR_URL);
    }
}
