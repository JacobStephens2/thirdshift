//! A Pickup run: from the Launch directory with no Issue URL, the search
//! for the Ready issue at its order's end of the issue numbers in the
//! repository, saying why it passed over each issue labelled
//! `ready-for-agent` before it, then that issue dispatched as a Spec run or
//! a Run. One started while its repository is at its Command limit on this
//! machine is skipped, before any search. One that runs first makes the
//! Sweep, takes `agent-failed` off each open issue the Day shift has acted
//! on, and is then skipped if it finds the repository at its Failure limit,
//! or with no Ready issue it can reserve: one another run reserved, a Pickup
//! run, or an Architect run or Security run dispatching its own plan or fix,
//! or whose selection went stale, is passed over for the next.

use std::fmt;
use std::num::NonZeroUsize;

use anyhow::Result;

use crate::asks::Flags;
use crate::claim;
use crate::command_limit::{AlreadyRunning, Sharing};
use crate::config::UserConfig;
use crate::github::{GitHub, ListedIssue};
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{self, Edit, Label, Labels, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::launch::{self, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{Dispatch, LaunchAndGitHub, Outside};
use crate::ready::{PickupOrder, ReadyIssue};
use crate::reservation::{self, Reservation};
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
    /// Whether the Ready issue was a Spec when taken.
    pub is_spec: bool,
    /// Its title, as the search listed it.
    pub title: String,
    /// How the Spec run or Run it was dispatched as ended.
    pub ended: Ended,
}

/// Why a Pickup run was skipped. Its `Display` is the reason, as the skipped
/// run's one line gives it.
pub enum Skipped {
    /// The repository is at its Command limit on this machine, each Pass's
    /// dispatched Spec run or Run counted as part of it.
    AlreadyRunning(AlreadyRunning),
    /// The repository is at its Failure limit: `failed` lists the open
    /// issues labelled `agent-failed` that still wait for the Day shift,
    /// lowest number first, and `limit` or more stop a Pickup run from
    /// taking another.
    AtFailureLimit {
        repo: Repo,
        failed: Vec<u64>,
        limit: NonZeroUsize,
    },
    /// The repository has no Ready issue.
    NoReadyIssue(Repo),
    /// Each Ready issue it found, `tried` in its order, was reserved by
    /// another run or ceased to be Ready once reserved, and none is
    /// left past them.
    NoneReservable { repo: Repo, tried: Vec<u64> },
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(running) => running.fmt(f),
            Self::AtFailureLimit {
                repo,
                failed,
                limit,
            } => {
                write!(f, "at the Failure limit on {}: ", repo.slug())?;
                write_numbers(f, failed)?;
                let verb = if failed.len() == 1 { "is" } else { "are" };
                write!(
                    f,
                    " {verb} labelled {}, pickup.failure_limit is {limit}",
                    claim::AGENT_FAILED
                )
            }
            Self::NoReadyIssue(repo) => write!(f, "no Ready issue on {}", repo.slug()),
            Self::NoneReservable { repo, tried } => {
                write!(f, "no Ready issue on {} it can reserve, past ", repo.slug())?;
                write_numbers(f, tried)?;
                f.write_str("; check ")?;
                for (index, number) in tried.iter().enumerate() {
                    let separator = if index == 0 { "" } else { ", " };
                    write!(f, "{separator}{}", reservation::reference(*number))?;
                }
                let stays = if tried.len() == 1 {
                    "it stays"
                } else {
                    "one stays"
                };
                write!(f, " on origin if {stays} reserved")
            }
        }
    }
}

/// `numbers` as "#7", "#7 and #9" or "#7, #8 and #9".
fn write_numbers(f: &mut fmt::Formatter, numbers: &[u64]) -> fmt::Result {
    for (index, number) in numbers.iter().enumerate() {
        let separator = if index == 0 {
            ""
        } else if index == numbers.len() - 1 {
            " and "
        } else {
            ", "
        };
        write!(f, "{separator}#{number}")?;
    }
    Ok(())
}

/// The line on Ready issue `number`, passed over because another run
/// reserved it or it ceased to be Ready once reserved.
fn unreservable(number: u64) -> String {
    format!(
        "#{number} is reserved by another run or is no longer Ready; \
         check {} on origin if it stays reserved",
        reservation::reference(number)
    )
}

/// Take the Ready issue at `order`'s end of the issue numbers in the Launch
/// directory's repository, saying which on stderr, after a line there on
/// each issue labelled `ready-for-agent` it passed over on the way, with
/// why, and dispatch it as `thirdshift <Issue URL>` with `flags` would run
/// it, but on the Pickup run's Base branch. The Base branch is `base`, the
/// branch the command named, whatever the Launch directory has checked out,
/// or without one the branch checked out there. Nothing is changed in the
/// Launch directory's checkout, nor on the issue taken: the run it is
/// dispatched as makes the Claim. An auxiliary origin ref reserves the
/// issue across hosts before dispatch; a stale Ready selection is rechecked
/// while it is held. An issue another run reserved, or whose selection went
/// stale, is passed over for the next Ready issue in `order`.
///
/// Once the preflight checks pass, and before any search, the Pickup run is
/// skipped if its repository is at `config`'s Command limit on this machine.
/// Otherwise this process holds a slot there until it exits, through
/// whatever it dispatches.
/// A skip is recorded in the repository's Activity log. The rest, from the
/// Sweep to the dispatch, is [`run_through`], with `config`'s Failure limit
/// and `order`, through the Launch directory, GitHub, the logs and
/// `harness`, the Harness, Model and Effort the run it dispatches will run
/// its sessions on, and `flags` and `config`, which ask that run.
pub fn run(
    base: Option<&str>,
    flags: &Flags,
    config: &UserConfig,
    harness: &mut Choice,
    order: PickupOrder,
) -> Result<Outcome> {
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base, config.command_limit, Sharing::UpToTheLimit)? {
        Start::Clear(launch) => launch,
        Start::AlreadyRunning(running) => {
            logs::skipped(Pass::Pickup, &running.repo, &running);
            return Ok(Outcome::Skipped(Skipped::AlreadyRunning(running)));
        }
    };
    // Only Pickup handles Base branch CI reporting. A CI or publication failure
    // must never prevent the usual Ready search or dispatch.
    if let Err(error) = handle_base_ci(directory.git(), &repo, base.name()) {
        crate::progress::warn(&error, format_args!("could not file CI-red Ticket"));
    }
    let mut outside = LaunchAndGitHub {
        launch: directory.git(),
        base: &base,
        repo: &repo,
        harness,
        flags,
        config,
        security_review_harness: None,
    };
    run_through(
        &mut outside,
        &repo,
        base.name(),
        config.pickup_failure_limit,
        order,
        |ready| {
            let Some(reservation) = Reservation::take(directory.git(), &ready.listed.issue)? else {
                return Ok(None);
            };
            Ok(
                crate::ready::refresh(directory.git(), ready, config.pickup_wait)?
                    .map(|ready| (ready, reservation)),
            )
        },
    )
}

fn handle_base_ci(git: &crate::git::Git, repo: &Repo, base: &str) -> Result<()> {
    let github = GitHub::new();
    let Some(_reservation) = Reservation::ci_red(git)? else {
        return Ok(());
    };
    let Some(failure) = github.base_ci_evidence(repo, base)? else {
        return Ok(());
    };
    match github.handle_base_ci(repo, failure)? {
        crate::github::BaseCiAction::Nothing | crate::github::BaseCiAction::RerunRequested => {}
        crate::github::BaseCiAction::CiRedFiled(issue) => {
            crate::progress::step(format!("filed CI-red Ticket {}", issue.url));
        }
        crate::github::BaseCiAction::FlakyFiled(issue) => {
            crate::progress::step(format!("filed Flaky-test Ticket {}", issue.url));
        }
        crate::github::BaseCiAction::Commented(issue) => {
            crate::progress::step(format!("commented new CI failure on {}", issue.url));
        }
    }
    Ok(())
}

/// [`run`], once it holds a slot on its repository, on `repo` with the Base
/// branch `base` and the Failure limit `limit`, through `outside`: its
/// [`gates`], then `reserve` acquiring cross-machine ownership and refreshing
/// the selection. An issue either refuses gets a line saying so, and the
/// search goes on past it in `order`, with a skip recorded once no Ready
/// issue is left. Then the line naming the issue taken; then the check of
/// its Harness, failing before any work if it can't run; then the record
/// that it started work on the issue, which keeps its Command log; then the
/// issue dispatched, a Spec run on a Spec and a Run otherwise, on `base`.
fn run_through<R>(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    limit: NonZeroUsize,
    order: PickupOrder,
    mut reserve: impl FnMut(ReadyIssue) -> Result<Option<(ReadyIssue, R)>>,
) -> Result<Outcome> {
    let mut ready = match gates(outside, repo, limit, order)? {
        Decision::Take(ready) => ready,
        Decision::Skip(skipped) => {
            outside.skipped(Pass::Pickup, &skipped);
            return Ok(Outcome::Skipped(skipped));
        }
    };
    let mut tried = Vec::new();
    // A reservation spans dispatch, including Claim ending and Failed run
    // preservation. No start is recorded and no Claim made by a losing host.
    // One refused is released before the next issue is tried.
    let (
        ReadyIssue {
            listed, is_spec, ..
        },
        _reservation,
    ) = loop {
        let number = ready.listed.issue.number;
        if let Some(taken) = reserve(ready)? {
            break taken;
        }
        outside.step(unreservable(number));
        tried.push(number);
        match outside.ready_issue(order, Some(number))? {
            Some(next) => ready = next,
            None => {
                let skipped = Skipped::NoneReservable {
                    repo: repo.clone(),
                    tried,
                };
                outside.skipped(Pass::Pickup, &skipped);
                return Ok(Outcome::Skipped(skipped));
            }
        }
    };
    outside.step(format!(
        "taking Ready issue #{} \"{}\", as thirdshift {} would",
        listed.issue.number, listed.title, listed.issue.url
    ));
    outside.check_harness(true)?;
    outside.started(Work::PickupRun(&listed));
    let ended = outside.dispatch(Dispatch::ReadyIssue {
        issue: &listed.issue,
        is_spec,
        base,
    });
    Ok(Outcome::Took(Took {
        issue: listed.issue,
        is_spec,
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
/// Sweep, then, if `limit` or more open issues still wait for the Day shift
/// after the factory failed on them, a skip for the Failure limit, then the
/// Ready issue search at `order`'s end of the issue numbers, a skip if it
/// finds none. No gate is read once one skips.
fn gates(
    outside: &mut impl Outside,
    repo: &Repo,
    limit: NonZeroUsize,
    order: PickupOrder,
) -> Result<Decision> {
    sweep(outside);
    let failed = clear_and_count_failures(outside)?;
    if failed.len() >= limit.get() {
        return Ok(Decision::Skip(Skipped::AtFailureLimit {
            repo: repo.clone(),
            failed,
            limit,
        }));
    }
    Ok(match outside.ready_issue(order, None)? {
        Some(ready) => Decision::Take(ready),
        None => Decision::Skip(Skipped::NoReadyIssue(repo.clone())),
    })
}

/// The labels the Sweep takes off closed issues.
const SWEPT: [Label; 2] = [claim::IN_PROGRESS, claim::AGENT_FAILED];

/// The Sweep: take `in-progress` and `agent-failed` off every closed issue
/// that still carries either, as an issue merged by hand does, leaving its
/// other labels. A failure, to list them or to take the labels off one, is
/// only a warning.
fn sweep(outside: &mut impl Outside) {
    let mut closed: Vec<ListedIssue> = Vec::new();
    for label in SWEPT {
        match outside.closed_issues(label) {
            Ok(listed) => {
                for listed in listed {
                    if !closed
                        .iter()
                        .any(|seen| seen.issue.number == listed.issue.number)
                    {
                        closed.push(listed);
                    }
                }
            }
            Err(error) => outside.step(format!(
                "warning: could not list the closed issues labelled {label}: {error:#}"
            )),
        }
    }
    for ListedIssue { issue, labels, .. } in closed {
        let edit = Edit::of(&issue, labels, &SWEPT, &[]);
        let taken_off = labels::joined(SWEPT.into_iter().filter(|label| edit.takes_off(*label)));
        if taken_off.is_empty() {
            continue;
        }
        outside.step(format!(
            "taking {taken_off} off #{}, which is closed",
            issue.number
        ));
        if let Err(error) = outside.apply(&edit) {
            outside.step(format!(
                "warning: could not take {taken_off} off #{}: {error:#}",
                issue.number
            ));
        }
    }
}

/// The open issues the factory failed on that still wait for the Day shift,
/// lowest number first: those labelled `agent-failed`, except one the Day
/// shift has acted on, labelled `ready-for-agent` again, or with neither
/// `in-progress` nor `needs-triage` left, which has `agent-failed` taken
/// off instead and is not counted. A failure to take it off is only a
/// warning; a failure to list them fails the Pickup run.
fn clear_and_count_failures(outside: &mut impl Outside) -> Result<Vec<u64>> {
    let mut failed = Vec::new();
    for ListedIssue { issue, labels, .. } in outside.open_issues(claim::AGENT_FAILED)? {
        if !labels.has(claim::AGENT_FAILED) {
            continue;
        }
        let Some(acted) = acted_on(&labels) else {
            failed.push(issue.number);
            continue;
        };
        outside.step(format!(
            "taking {} off #{}, which the Day shift {acted}",
            claim::AGENT_FAILED,
            issue.number
        ));
        let edit = Edit::of(&issue, labels, &[claim::AGENT_FAILED], &[]);
        if let Err(error) = outside.apply(&edit) {
            outside.step(format!(
                "warning: could not take {} off #{}: {error:#}",
                claim::AGENT_FAILED,
                issue.number
            ));
        }
    }
    failed.sort_unstable();
    Ok(failed)
}

/// How the Day shift acted on the failure of an open issue with `labels`, if
/// it did: handed it back with `ready-for-agent`, or took it over, leaving
/// it with neither `in-progress` nor `needs-triage`.
fn acted_on(labels: &Labels) -> Option<&'static str> {
    if labels.has(READY_FOR_AGENT) {
        Some("handed back")
    } else if !claim::is_on(labels) && !labels.has(NEEDS_TRIAGE) {
        Some("took over")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, PR_URL, ready_for_review, widgets};

    /// The gates of a Pickup run on `repo` with the Failure limit `limit`:
    /// the line of the reason it is skipped, or the number of the Ready
    /// issue it takes and whether it is a Spec, and what the gates did.
    fn decide(mut repo: InMemory, limit: usize) -> (Result<(u64, bool), String>, Vec<Call>) {
        let limit = NonZeroUsize::new(limit).unwrap();
        let decided = match gates(&mut repo, &widgets(), limit, PickupOrder::LowestFirst).unwrap() {
            Decision::Take(ready) => Ok((ready.listed.issue.number, ready.is_spec)),
            Decision::Skip(skipped) => Err(skipped.to_string()),
        };
        (decided, repo.calls)
    }

    const CLOSED: Call = Call::Closed("in-progress");
    const CLOSED_FAILED: Call = Call::Closed("agent-failed");
    const FAILED: Call = Call::Open("agent-failed");

    /// The Sweep's listings, closed `in-progress` then `agent-failed`.
    fn listing_closed() -> [Call; 2] {
        [CLOSED, CLOSED_FAILED]
    }

    /// The Sweep taking `in-progress` off issue `number`, leaving `labels`.
    fn swept(number: u64, labels: &[&str]) -> [Call; 2] {
        swept_off(number, &["in-progress"], labels)
    }

    /// The Sweep taking `off` off issue `number`, leaving `labels`.
    fn swept_off(number: u64, off: &[&'static str], labels: &[&str]) -> [Call; 2] {
        [
            Call::Step(format!(
                "taking {} off #{number}, which is closed",
                off.join(" and ")
            )),
            Call::Edit {
                issue: number,
                off: off.to_vec(),
                on: vec![],
                labels: labels.iter().map(|label| label.to_string()).collect(),
            },
        ]
    }

    /// The gates' calls up to the count: the Sweep's listings, with
    /// nothing swept, then the listing of the open `agent-failed` issues.
    fn counted() -> Vec<Call> {
        vec![CLOSED, CLOSED_FAILED, FAILED]
    }

    const NO_READY_ISSUE: &str = "no Ready issue on acme/widgets";

    #[test]
    fn the_ready_issue_is_taken_with_its_spec_or_not_answer() {
        for is_spec in [false, true] {
            let repo = InMemory::default().ready(9, is_spec);

            let (decided, calls) = decide(repo, 3);

            assert_eq!(decided, Ok((9, is_spec)));
            let mut expected = counted();
            expected.push(Call::ReadySearch);
            assert_eq!(calls, expected);
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
        let mut expected = Vec::from(listing_closed());
        expected.extend(swept(4, &["bug", "p1"]));
        expected.extend(swept(5, &[]));
        expected.extend([FAILED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_skips_a_closed_issue_its_edit_would_take_nothing_off() {
        let repo = InMemory::default()
            .filed("in-progress", 4, false, &["bug"])
            .issue(5, false, &["in-progress"])
            .ready(9, false);

        let (_, calls) = decide(repo, 3);

        let mut expected = Vec::from(listing_closed());
        expected.extend(swept(5, &[]));
        expected.extend([FAILED, Call::ReadySearch]);
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
        let warning = |label: &str| {
            Call::Step(format!(
                "warning: could not list the closed issues labelled {label}: \
                 gh: could not list the closed issues"
            ))
        };
        assert_eq!(
            calls,
            [
                CLOSED,
                warning("in-progress"),
                CLOSED_FAILED,
                warning("agent-failed"),
                FAILED,
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
        let mut expected = Vec::from(listing_closed());
        expected.extend(swept(4, &[]));
        expected.push(Call::Step(
            "warning: could not take in-progress off #4: gh: could not edit #4".to_string(),
        ));
        expected.extend(swept(5, &[]));
        expected.extend([FAILED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_takes_agent_failed_off_each_closed_issue_once_with_in_progress() {
        let repo = InMemory::default()
            .issue(4, false, &["bug", "in-progress", "agent-failed"])
            .issue(5, false, &["Agent-Failed", "p1"])
            .issue(6, true, &["agent-failed", "in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = Vec::from(listing_closed());
        expected.extend(swept_off(4, &["in-progress", "agent-failed"], &["bug"]));
        expected.extend(swept_off(5, &["agent-failed"], &["p1"]));
        expected.extend([FAILED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_comes_before_the_count_on_a_pass_skipped_for_the_failure_limit() {
        let repo = InMemory::default()
            .issue(4, false, &["in-progress"])
            .issue(5, true, &["in-progress", "agent-failed"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 1);

        assert!(decided.is_err());
        let mut expected = Vec::from(listing_closed());
        expected.extend(swept(4, &[]));
        expected.push(FAILED);
        assert_eq!(calls, expected);
    }

    #[test]
    fn the_sweep_comes_before_the_count_on_a_pass_skipped_for_no_ready_issue() {
        let repo = InMemory::default().issue(4, false, &["in-progress"]);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Err(NO_READY_ISSUE.to_string()));
        let mut expected = Vec::from(listing_closed());
        expected.extend(swept(4, &[]));
        expected.extend([FAILED, Call::ReadySearch]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn at_exactly_the_failure_limit_the_pass_is_skipped_with_no_ready_issue_search() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress", "agent-failed"])
            .issue(5, true, &["In-Progress", "bug", "Agent-Failed"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 2);

        assert_eq!(
            decided,
            Err(
                "at the Failure limit on acme/widgets: #4 and #5 are labelled agent-failed, \
                 pickup.failure_limit is 2"
                    .to_string()
            )
        );
        assert_eq!(calls, counted());
    }

    #[test]
    fn over_the_failure_limit_the_pass_is_skipped_naming_every_failure_lowest_first() {
        let repo = InMemory::default()
            .issue(6, true, &["in-progress", "agent-failed"])
            .issue(4, true, &["in-progress", "agent-failed"])
            .issue(5, true, &["needs-triage", "agent-failed"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 2);

        assert_eq!(
            decided,
            Err(
                "at the Failure limit on acme/widgets: #4, #5 and #6 are labelled agent-failed, \
                 pickup.failure_limit is 2"
                    .to_string()
            )
        );
        assert_eq!(calls, counted());
    }

    #[test]
    fn one_below_the_failure_limit_the_pass_takes_the_ready_issue() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress", "agent-failed"])
            .issue(5, true, &["in-progress", "agent-failed"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = counted();
        expected.push(Call::ReadySearch);
        assert_eq!(calls, expected);
    }

    #[test]
    fn claims_whose_pull_requests_wait_for_review_do_not_count() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress"])
            .issue(5, true, &["in-progress"])
            .issue(6, true, &["in-progress"])
            .ready(9, false);

        let (decided, _) = decide(repo, 1);

        assert_eq!(decided, Ok((9, false)));
    }

    /// The count taking `agent-failed` off open issue `number`, which the
    /// Day shift `acted` on, leaving `labels`.
    fn handed_back(number: u64, acted: &str, labels: &[&str]) -> [Call; 2] {
        [
            Call::Step(format!(
                "taking agent-failed off #{number}, which the Day shift {acted}"
            )),
            Call::Edit {
                issue: number,
                off: vec!["agent-failed"],
                on: vec![],
                labels: labels.iter().map(|label| label.to_string()).collect(),
            },
        ]
    }

    #[test]
    fn a_failure_the_day_shift_acted_on_has_agent_failed_taken_off_and_is_not_counted() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress", "agent-failed", "Ready-For-Agent"])
            .issue(5, true, &["agent-failed", "ready-for-agent"])
            .issue(6, true, &["agent-failed", "bug"])
            .issue(7, true, &["agent-failed", "needs-triage"])
            .issue(8, true, &["in-progress", "agent-failed"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 3);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = counted();
        expected.extend(handed_back(
            4,
            "handed back",
            &["in-progress", "Ready-For-Agent"],
        ));
        expected.extend(handed_back(5, "handed back", &["ready-for-agent"]));
        expected.extend(handed_back(6, "took over", &["bug"]));
        expected.push(Call::ReadySearch);
        assert_eq!(calls, expected);
    }

    #[test]
    fn a_failure_whose_label_cannot_be_taken_off_warns_and_is_still_not_counted() {
        let repo = InMemory::default()
            .issue(4, true, &["agent-failed", "ready-for-agent"])
            .ready(9, false)
            .edit_failing(4);

        let (decided, calls) = decide(repo, 1);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = counted();
        expected.extend(handed_back(4, "handed back", &["ready-for-agent"]));
        expected.push(Call::Step(
            "warning: could not take agent-failed off #4: gh: could not edit #4".to_string(),
        ));
        expected.push(Call::ReadySearch);
        assert_eq!(calls, expected);
    }

    #[test]
    fn an_issue_listed_as_agent_failed_that_no_longer_has_it_is_not_counted() {
        let repo = InMemory::default()
            .filed("agent-failed", 4, true, &["in-progress"])
            .ready(9, false);

        let (decided, calls) = decide(repo, 1);

        assert_eq!(decided, Ok((9, false)));
        let mut expected = counted();
        expected.push(Call::ReadySearch);
        assert_eq!(calls, expected);
    }

    /// A Pickup run on `repo`, past its slot, on the Base branch `main` with
    /// the Failure limit 3, its dispatched run ending ready for review: how
    /// it ended, and what it did outside itself.
    fn pickup(repo: InMemory) -> (Result<Outcome>, Vec<Call>) {
        let mut repo = repo.dispatched_ending(ready_for_review());
        let limit = NonZeroUsize::new(3).unwrap();
        let outcome = run_through(
            &mut repo,
            &widgets(),
            "main",
            limit,
            PickupOrder::LowestFirst,
            |ready| Ok(Some((ready, ()))),
        );
        (outcome, repo.calls)
    }

    /// [`pickup`] in `order`, with `reserve` reserving each Ready issue
    /// the run tries: how it ended, and what it did outside itself.
    fn pickup_reserving(
        repo: InMemory,
        order: PickupOrder,
        reserve: impl FnMut(ReadyIssue) -> Result<Option<(ReadyIssue, ())>>,
    ) -> (Result<Outcome>, Vec<Call>) {
        let mut repo = repo.dispatched_ending(ready_for_review());
        let limit = NonZeroUsize::new(3).unwrap();
        let outcome = run_through(&mut repo, &widgets(), "main", limit, order, reserve);
        (outcome, repo.calls)
    }

    /// A reservation of each Ready issue but those numbered in `refused`,
    /// which another Pickup run holds or whose selection went stale.
    fn refusing(refused: &[u64]) -> impl FnMut(ReadyIssue) -> Result<Option<(ReadyIssue, ())>> {
        move |ready| Ok((!refused.contains(&ready.listed.issue.number)).then_some((ready, ())))
    }

    /// The line on Ready issue `number`, which the run could not reserve.
    fn unreservable_line(number: u64) -> Call {
        Call::Step(unreservable(number))
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
        let mut expected = counted();
        expected.extend([Call::ReadySearch, Call::Skipped(NO_READY_ISSUE.to_string())]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn a_pass_skipped_at_the_failure_limit_records_its_reason_and_does_no_work() {
        let repo = InMemory::default()
            .issue(4, true, &["in-progress", "agent-failed"])
            .issue(5, true, &["in-progress", "agent-failed"])
            .issue(6, true, &["in-progress", "agent-failed"])
            .ready(9, false);

        let (outcome, calls) = pickup(repo);

        assert!(matches!(
            outcome,
            Ok(Outcome::Skipped(Skipped::AtFailureLimit { .. }))
        ));
        let mut expected = counted();
        expected.push(Call::Skipped(
            "at the Failure limit on acme/widgets: #4, #5 and #6 are labelled agent-failed, \
             pickup.failure_limit is 3"
                .to_string(),
        ));
        assert_eq!(calls, expected);
    }

    #[test]
    fn a_pass_that_takes_an_issue_names_it_checks_its_harness_starts_then_dispatches_it() {
        let (_, calls) = pickup(InMemory::default().ready(9, false));

        let mut expected = counted();
        expected.extend([
            Call::ReadySearch,
            taking_9(),
            Call::HarnessCheck,
            Call::Started(Some(9)),
            Call::DispatchReadyIssue {
                issue: 9,
                is_spec: false,
                base: "main".to_string(),
            },
        ]);
        assert_eq!(calls, expected);
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
        assert_eq!(reached.resolution.url(), PR_URL);
    }

    #[test]
    fn a_ready_issue_it_cannot_reserve_is_passed_over_for_the_next_in_its_order() {
        let repo = InMemory::default().ready(7, false).ready(9, false);

        let (outcome, calls) = pickup_reserving(repo, PickupOrder::LowestFirst, refusing(&[7]));

        assert!(matches!(outcome, Ok(Outcome::Took(Took { ref issue, .. })) if issue.number == 9));
        assert_eq!(
            calls,
            [
                CLOSED,
                CLOSED_FAILED,
                FAILED,
                Call::ReadySearch,
                unreservable_line(7),
                Call::ReadySearchPast(7),
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
    fn highest_first_moves_on_downward_from_a_ready_issue_it_cannot_reserve() {
        let repo = InMemory::default()
            .ready(7, false)
            .ready(8, false)
            .ready(9, false);

        let (outcome, calls) = pickup_reserving(repo, PickupOrder::HighestFirst, refusing(&[9, 8]));

        assert!(matches!(outcome, Ok(Outcome::Took(Took { ref issue, .. })) if issue.number == 7));
        assert_eq!(
            calls[3..8],
            [
                Call::ReadySearch,
                unreservable_line(9),
                Call::ReadySearchPast(9),
                unreservable_line(8),
                Call::ReadySearchPast(8),
            ]
        );
    }

    #[test]
    fn with_no_ready_issue_it_can_reserve_left_the_pass_is_skipped_naming_each_tried() {
        let repo = InMemory::default().ready(7, false).ready(9, false);

        let (outcome, calls) = pickup_reserving(repo, PickupOrder::LowestFirst, refusing(&[7, 9]));

        assert!(matches!(
            outcome,
            Ok(Outcome::Skipped(Skipped::NoneReservable { ref tried, .. })) if tried == &[7, 9]
        ));
        assert_eq!(
            calls,
            [
                CLOSED,
                CLOSED_FAILED,
                FAILED,
                Call::ReadySearch,
                unreservable_line(7),
                Call::ReadySearchPast(7),
                unreservable_line(9),
                Call::ReadySearchPast(9),
                Call::Skipped(
                    "no Ready issue on acme/widgets it can reserve, past #7 and #9; check \
                     refs/heads/thirdshift/pickup/7, refs/heads/thirdshift/pickup/9 on origin \
                     if one stays reserved"
                        .to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_reservation_that_fails_on_the_next_ready_issue_fails_the_pass_before_any_work() {
        let repo = InMemory::default().ready(7, false).ready(9, false);

        let (outcome, calls) = pickup_reserving(repo, PickupOrder::LowestFirst, |ready| {
            if ready.listed.issue.number == 9 {
                anyhow::bail!("could not reserve refs/heads/thirdshift/pickup/9");
            }
            Ok(None)
        });

        let error = outcome.err().expect("the pass did not fail");
        assert_eq!(
            format!("{error:#}"),
            "could not reserve refs/heads/thirdshift/pickup/9"
        );
        assert_eq!(calls.last(), Some(&Call::ReadySearchPast(7)));
    }
}
