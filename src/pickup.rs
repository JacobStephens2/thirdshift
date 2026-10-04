//! A Pickup run up to the issue it takes: from the Launch directory with no
//! Issue URL, the search for the lowest-numbered Ready issue in the
//! repository, for the command to dispatch, saying why it passed over each
//! issue labelled `ready-for-agent` before it. Only one Pickup run or
//! Architect run per repository runs at a time on a machine: one started
//! while another is still running is skipped, before any search. One that
//! runs first makes the Sweep, and is then skipped if it finds the
//! repository at its Claim limit, or with no Ready issue.

use std::fmt;
use std::num::NonZeroUsize;

use anyhow::Result;

use crate::claim;
use crate::github::ListedIssue;
use crate::issue::{IssueUrl, Repo};
use crate::labels::Edit;
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{OnGitHub, Outside};
use crate::progress;
use crate::ready::ReadyIssue;

/// How a Pickup run ended, short of a failure and before any dispatch.
pub enum Outcome {
    /// It took this Ready issue, to dispatch.
    Taken(Taken),
    /// Skipped, having done nothing.
    Skipped(Skipped),
}

/// The Ready issue a Pickup run took.
pub struct Taken {
    pub issue: IssueUrl,
    /// Its title, as the search listed it.
    pub title: String,
    /// The Pickup run's Base branch, which the run the issue is dispatched
    /// as takes.
    pub base: String,
    /// Whether it is a Spec, an issue with sub-issues, which is dispatched
    /// as a Spec run.
    pub is_spec: bool,
}

/// Why a Pickup run was skipped. Its `Display` is the reason, as the skipped
/// run's one line gives it.
pub enum Skipped {
    /// An Architect run or another Pickup run on this repository is still
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

impl Skipped {
    /// The repository the Pickup run was skipped on.
    fn repo(&self) -> &Repo {
        match self {
            Self::AlreadyRunning(AlreadyRunning(repo))
            | Self::AtClaimLimit { repo, .. }
            | Self::NoReadyIssue(repo) => repo,
        }
    }
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
/// labelled `ready-for-agent` it passed over on the way, with why. The Base
/// branch is `base`, the branch the command named, whatever the Launch
/// directory has checked out, or without one the branch checked out there.
/// Nothing is changed in the Launch directory, nor on the issue taken: the
/// run it is dispatched as makes the Claim.
///
/// Once the preflight checks pass, and before any search, the Pickup run is
/// skipped if an Architect run or another Pickup run on the same repository
/// is still running on this machine. Otherwise this process is that
/// repository's one such run until it exits, through whatever it dispatches.
/// It first makes the Sweep, taking the Claim off the repository's closed
/// issues, and is then skipped, still before any search, if the repository
/// is at its Claim limit: `limit` or more of its open issues carry a Claim.
/// A skip is recorded in the repository's Activity log. Once it takes an
/// issue, it records that it started work on it, which keeps its Command
/// log.
pub fn run(base: Option<&str>, limit: NonZeroUsize) -> Result<Outcome> {
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base)? {
        Start::Clear(launch) => launch,
        Start::AlreadyRunning(running) => {
            return Ok(skip(Skipped::AlreadyRunning(running)));
        }
    };
    let mut on_github = OnGitHub {
        launch: directory.git(),
        repo: &repo,
    };
    let ReadyIssue { listed, is_spec } = match gates(&mut on_github, &repo, limit)? {
        Decision::Take(ready) => ready,
        Decision::Skip(skipped) => return Ok(skip(skipped)),
    };
    progress::step(format_args!(
        "taking Ready issue #{} \"{}\", as thirdshift {} would",
        listed.issue.number, listed.title, listed.issue.url
    ));
    logs::started(Work::PickupRun(&listed.issue));
    Ok(Outcome::Taken(Taken {
        issue: listed.issue,
        title: listed.title,
        base,
        is_spec,
    }))
}

/// What a Pickup run's gates decided.
enum Decision {
    /// It is skipped, for this reason.
    Skip(Skipped),
    /// It takes this Ready issue.
    Take(ReadyIssue),
}

/// The gates of a Pickup run on `repo`, reaching it through `pass`: the
/// Sweep, then, if `limit` or more of its open issues carry a Claim, a skip
/// for the Claim limit, then the Ready issue search, a skip if it finds
/// none. No gate is passed through once one skips.
fn gates(pass: &mut impl Outside, repo: &Repo, limit: NonZeroUsize) -> Result<Decision> {
    sweep(pass);
    let claimed = pass.open_issues(claim::IN_PROGRESS)?.len();
    if claimed >= limit.get() {
        return Ok(Decision::Skip(Skipped::AtClaimLimit {
            repo: repo.clone(),
            claimed,
            limit,
        }));
    }
    Ok(match pass.ready_issue()? {
        Some(ready) => Decision::Take(ready),
        None => Decision::Skip(Skipped::NoReadyIssue(repo.clone())),
    })
}

/// The Sweep: take `in-progress` off every closed issue that still carries
/// it, as an issue merged by hand does, leaving its other labels. A
/// failure, to list them or to take the label off one, is only a warning.
fn sweep(pass: &mut impl Outside) {
    let closed = match pass.closed_issues(claim::IN_PROGRESS) {
        Ok(closed) => closed,
        Err(error) => {
            pass.step(format!(
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
        pass.step(format!(
            "taking {} off #{}, which is closed",
            claim::IN_PROGRESS,
            issue.number
        ));
        if let Err(error) = pass.apply(&edit) {
            pass.step(format!(
                "warning: could not take {} off #{}: {error:#}",
                claim::IN_PROGRESS,
                issue.number
            ));
        }
    }
}

/// The Pickup run skipped as `skipped` says, recorded in its repository's
/// Activity log.
fn skip(skipped: Skipped) -> Outcome {
    logs::skipped(Pass::PickupRun, skipped.repo(), &skipped);
    Outcome::Skipped(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory};

    /// The repository every pass is on.
    fn widgets() -> Repo {
        Repo {
            owner: "acme".to_string(),
            name: "widgets".to_string(),
        }
    }

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
        let mut repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .ready(9, false);
        repo.closed_listing_fails = true;

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
        let mut repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .issue(5, false, &["in-progress"])
            .ready(9, false);
        repo.failing_edits = vec![4];

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
    fn one_below_the_claim_limit_the_pass_takes_the_ready_issue() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        assert_eq!(calls, [CLOSED, CLAIMED, Call::ReadySearch]);
    }
}
