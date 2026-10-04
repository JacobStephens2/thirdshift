//! One Run: from an Issue URL to a checked PR, or to a Failed run. An issue
//! with sub-issues is a Spec instead, handed to a Spec run.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Context, anyhow};

use crate::asks::Asks;
use crate::base_fix::{Advice, BaseFix};
use crate::branch::{self, Selection};
use crate::child_run::Kind;
use crate::claim;
use crate::delivery::{Delivery, Opening};
use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::{self, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::logs::{self, Work};
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::session::Logs;
use crate::spec_run;
use crate::worktree::Worktree;

/// Where a Run takes its PR: ready for review, or, in a Merge run, merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    ReadyForReview,
    Merged,
}

impl Goal {
    /// What became of the PR once the Run reached this goal, as in
    /// "PR <url> is merged" and a Run notification's subject.
    pub fn outcome(self) -> &'static str {
        match self {
            Goal::ReadyForReview => "ready for review",
            Goal::Merged => "merged",
        }
    }
}

/// A Run, or a Spec run, that reached its goal.
pub struct Reached {
    pub pr_url: String,
    /// The goal reached.
    pub goal: Goal,
    /// The most recent session's log, if a session was started.
    pub log: Option<PathBuf>,
    /// In a Spec run, a line on each Ticket it landed; empty in a Run.
    pub ticket_lines: Vec<String>,
}

/// How a Run, or a Spec run, ended.
pub struct Ended {
    pub outcome: Result<Reached, FailedRun>,
    /// What became of the Base fix it started or waited on, if any, as
    /// [`BaseFix::report`] tells it.
    pub base_fix: Option<String>,
    /// What it says after its cause, if Inherited failures failed it with no
    /// Base fix taken, as [`BaseFix::into_advice`] gives it.
    pub advice: Vec<Advice>,
}

/// What started a Run, or a Spec run, which says where its Base branch comes
/// from when it isn't a Continuation's open pull request that says.
#[derive(Clone, Copy)]
pub enum StartedBy<'a> {
    /// `thirdshift <Issue URL>`: the Base branch is the branch checked out
    /// in the Launch directory.
    Command,
    /// An Architect run, dispatching its plan, or a Pickup run, dispatching
    /// the Ready issue it took: the Base branch is that run's, whatever the
    /// Launch directory has checked out.
    Dispatch { base: &'a str },
    /// Another thirdshift, as this child Run, a Ticket's Run in a Spec run or
    /// a Base fix: the Base branch is the one the child Run was given.
    Child(&'a Kind),
}

impl<'a> StartedBy<'a> {
    /// The child Run this is, if another thirdshift started it.
    fn child(self) -> Option<&'a Kind> {
        match self {
            StartedBy::Child(kind) => Some(kind),
            StartedBy::Command | StartedBy::Dispatch { .. } => None,
        }
    }

    /// Whether the Run, or the Spec run, makes the Claim on its issue: a
    /// child Run makes none, so that only the issue the Day shift would look
    /// at carries one.
    fn makes_claim(self) -> bool {
        self.child().is_none()
    }

    /// The Base branch the Run was given, if what started it gave one.
    fn given_base(self) -> Option<&'a str> {
        match self {
            StartedBy::Command => None,
            StartedBy::Dispatch { base } => Some(base),
            StartedBy::Child(kind) => Some(kind.base()),
        }
    }
}

/// [`run`] the Run on `issue` that `started_by` started and that is asked
/// `asks`, to its end. Its Run notification, if `asks` has it send one, is
/// for what started the Run to send, once it has ended.
pub fn run_to_end(issue: &IssueUrl, asks: &Asks, started_by: StartedBy) -> Ended {
    let mut base_fix = BaseFix::new(started_by.child(), asks.base_fix.clone());
    let outcome = run(issue, asks, started_by, &mut base_fix);
    Ended {
        outcome,
        base_fix: base_fix.report(),
        advice: base_fix.into_advice(),
    }
}

/// Take `issue` to a ready PR, or in a Merge run a merged one. Any failure
/// after the worktree exists, including a merge that fails, goes through the
/// Failed run path. The worktree, the local
/// Issue branch and the plugin directory are gone when this returns, except
/// that a Failed run whose work did not reach origin keeps the worktree and
/// branch. With `asks.launch_pull`, the Launch directory's checkout of the
/// Base branch, if that is the branch checked out, is first brought up to
/// date with origin.
///
/// The Base branch `started_by` gave the Run, if it gave one, stands in for
/// the checked-out branch as the Base branch: the Spec branch or the Base
/// branch of the Run that started a child Run, or the Base branch of the
/// Architect run or the Pickup run that dispatched this one. Unless the Run
/// is a child Run, an issue with sub-issues is a Spec, taken on by a Spec run
/// instead, whose Spec branch is picked like an Issue branch, running as many
/// Tickets at once as `asks.tickets_at_once` says. A `parallel` the command
/// asked for, as `asks.parallel_asked` says, on an issue with no sub-issues
/// fails before any work, as does a Spec whose Tickets are all closed with
/// no Spec branch to continue.
///
/// Once those checks pass, and before the worktree is created, the Run, or
/// the Spec run, makes the Claim on `issue`, unless it is a child Run. A Claim
/// that can't be made fails it there, before any work. The Claim is released
/// if it then fails with nothing on origin to take over, and removed once its
/// Self-merge has left `issue` closed: see [`claim::Claim`].
///
/// `base_fix` is the one Base fix the Run, or a Spec run for its Spec PR, may
/// start, or wait on, when its only red checks are Inherited failures: what
/// `asks` ask about one is already in it.
fn run(
    issue: &IssueUrl,
    asks: &Asks,
    started_by: StartedBy,
    base_fix: &mut BaseFix,
) -> Result<Reached, FailedRun> {
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    preflight::check(&launch, issue)?;
    let tickets = match started_by.child() {
        Some(_) => Vec::new(),
        None => github::tickets(issue)?,
    };
    logs::started(match tickets.is_empty() {
        true => Work::Run(issue),
        false => Work::SpecRun(issue),
    });
    if asks.parallel_asked && tickets.is_empty() {
        return Err(anyhow!(
            "parallel is only for a Spec, and #{} has no sub-issues",
            issue.number
        )
        .into());
    }
    let checked_out = launch
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok();
    let selection = branch::select(&launch, issue)?;
    // A Spec implemented some other way: the Spec run leaves it be.
    if matches!(selection, Selection::Fresh { .. })
        && spec_run::all_closed(tickets.iter().map(|ticket| ticket.is_open))
    {
        return Err(
            anyhow!("every Ticket is closed and there is no Spec branch; nothing to do").into(),
        );
    }
    let base = selection.base_branch(started_by.given_base(), checked_out.as_deref())?;
    preflight::check_base_branch(&launch, &base)?;
    if asks.launch_pull {
        pull_base_branch(&launch, checked_out.as_deref(), &base);
    }

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let claim = started_by
        .makes_claim()
        .then(|| claim::make(issue))
        .transpose()?;
    let logs = Logs::of_run(issue);
    let delivery = Delivery {
        issue,
        base: &base,
        goal: asks.goal,
        base_fix,
        logs: &logs,
    };
    let outcome = run_in_worktree(tickets, &launch, &selection, delivery, asks.tickets_at_once);
    if let Some(claim) = claim {
        match &outcome {
            Ok(reached) if reached.goal == Goal::Merged => claim.remove_if_closed(),
            // Ready for review: the Claim stays while the pull request waits.
            Ok(_) => {}
            Err(_) => claim.release_if_nothing_on_origin(&launch),
        }
    }
    outcome
}

/// [`run`], from the worktree on: create it in the Launch directory `launch`
/// for the branch `selection` picked, and take the issue there by `delivery`,
/// as a Spec run, running up to `parallel` at once, if it has `tickets`.
fn run_in_worktree(
    tickets: Vec<Ticket>,
    launch: &Git,
    selection: &Selection,
    delivery: Delivery,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let (issue, base) = (delivery.issue, delivery.base);
    let branch = selection.branch();
    let worktree = match selection {
        Selection::Fresh { .. } => Worktree::create_fresh(launch, &issue.repo, branch, base)?,
        Selection::Continuation { .. } => {
            Worktree::continue_existing(launch, &issue.repo, branch, base)?
        }
    };
    if !tickets.is_empty() {
        return spec_run::run(tickets, worktree, delivery, parallel);
    }
    let prompt = match selection {
        Selection::Fresh { .. } => prompt::fresh(issue, base, branch),
        Selection::Continuation { pr, .. } => {
            prompt::continuation(issue, base, branch, pr.as_ref().map(|pr| pr.url.as_str()))
        }
    };
    let opening = Opening {
        kind: "implement",
        prompt,
        catch_up_from_origin: false,
    };
    delivery.deliver(worktree, opening, || Ok(()))
}

/// Fast-forward the Launch directory's Base branch `base` to
/// `origin/<base>`, if `base` is the branch `checked_out` there. Call it after
/// [`preflight::check_base_branch`], which fetches `origin/<base>` and fails
/// if `base` is ahead of it. The Run doesn't depend on this, so a failure,
/// such as uncommitted changes in the way, is only a warning, and those
/// changes are left as they were.
pub fn pull_base_branch(launch: &Git, checked_out: Option<&str>, base: &str) {
    if checked_out != Some(base) {
        return;
    }
    let origin_base = format!("origin/{base}");
    let up_to_date = launch.succeeds(&["merge-base", "--is-ancestor", &origin_base, "HEAD"]);
    if up_to_date.unwrap_or(false) {
        return;
    }
    progress::step(format_args!(
        "updating {base} in the Launch directory from {origin_base}"
    ));
    if let Err(error) = launch.run(&["merge", "--ff-only", "--quiet", &origin_base]) {
        progress::warn(
            &error,
            format_args!(
                "could not update {base} in the Launch directory, \
                 so update it by hand: git pull --ff-only origin {base}"
            ),
        );
    }
}
