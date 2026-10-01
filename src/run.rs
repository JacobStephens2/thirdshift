//! One Run: from an Issue URL to a checked PR, or to a Failed run. An issue
//! with sub-issues is a Spec instead, handed to a Spec run.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::base_fix::{Advice, BaseFix, BaseFixAsk};
use crate::branch::{self, Selection};
use crate::child_run::Kind;
use crate::ci::{self, Ci};
use crate::claim;
use crate::failed_run::{self, FailedRun, PolicyRefusal};
use crate::git::Git;
use crate::github::{self, Mergeable, PullRequest, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::poll;
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::session::{Logs, Sessions};
use crate::spec_run::{self, Parallel};
use crate::worktree::{Merge, Worktree};

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

/// [`run`] the Run on `issue` that `started_by` started and that asked
/// `base_fix` about a Base fix, to its end.
pub fn run_to_end(
    issue: &IssueUrl,
    goal: Goal,
    logs_dir: &Path,
    launch_pull: bool,
    parallel: Parallel,
    started_by: StartedBy,
    base_fix: BaseFixAsk,
) -> Ended {
    let mut base_fix = BaseFix::new(started_by.child(), base_fix);
    let outcome = run(
        issue,
        goal,
        logs_dir,
        launch_pull,
        parallel,
        started_by,
        &mut base_fix,
    );
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
/// branch. With `launch_pull`, the Launch directory's checkout of the
/// Base branch, if that is the branch checked out, is first brought up to
/// date with origin.
///
/// The Base branch `started_by` gave the Run, if it gave one, stands in for
/// the checked-out branch as the Base branch: the Spec branch or the Base
/// branch of the Run that started a child Run, or the Base branch of the
/// Architect run or the Pickup run that dispatched this one. Unless the Run
/// is a child Run, an issue with sub-issues is a Spec, taken on by a Spec run
/// instead, whose Spec branch is picked like an Issue branch, running as many
/// Tickets at once as `parallel` says. A `parallel` the command asked for on an issue
/// with no sub-issues fails before any work, as does a Spec whose Tickets are
/// all closed with no Spec branch to continue.
///
/// Once those checks pass, and before the worktree is created, the Run, or
/// the Spec run, makes the Claim on `issue`, unless it is a child Run. A Claim
/// that can't be made fails it there, before any work. The Claim is released
/// if it then fails with nothing on origin to take over, and removed once its
/// Self-merge has left `issue` closed: see [`claim::Claim`].
///
/// `base_fix` is the one Base fix the Run, or a Spec run for its Spec PR, may
/// start, or wait on, when its only red checks are Inherited failures.
fn run(
    issue: &IssueUrl,
    goal: Goal,
    logs_dir: &Path,
    launch_pull: bool,
    parallel: Parallel,
    started_by: StartedBy,
    base_fix: &mut BaseFix,
) -> Result<Reached, FailedRun> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    preflight::check(&launch, issue)?;
    let tickets = match started_by.child() {
        Some(_) => Vec::new(),
        None => github::tickets(issue)?,
    };
    if parallel.asked && tickets.is_empty() {
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
    if matches!(selection, Selection::Fresh { .. }) && spec_run::all_closed(&tickets) {
        return Err(
            anyhow!("every Ticket is closed and there is no Spec branch; nothing to do").into(),
        );
    }
    let branch = selection.branch().to_string();
    let base = selection.base_branch(started_by.given_base(), checked_out.as_deref())?;
    preflight::check_base_branch(&launch, &base)?;
    if launch_pull {
        pull_base_branch(&launch, checked_out.as_deref(), &base);
    }

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let claim = started_by
        .makes_claim()
        .then(|| claim::make(issue))
        .transpose()?;
    let logs = Logs::of_run(issue, logs_dir, &timestamp);
    let outcome = run_in_worktree(
        issue,
        tickets,
        &launch,
        &selection,
        &base,
        goal,
        base_fix,
        &logs,
        parallel.tickets,
    );
    if let Some(claim) = claim {
        match &outcome {
            Ok(reached) if reached.goal == Goal::Merged => claim.remove_if_closed(),
            // Ready for review: the Claim stays while the pull request waits.
            Ok(_) => {}
            Err(_) => claim.release_if_nothing_on_origin(&launch, &branch),
        }
    }
    outcome
}

/// [`run`], from the worktree on: create it in the Launch directory `launch`
/// for the branch `selection` picked, and take `issue` to `goal` there, as a
/// Spec run if it has `tickets`.
#[allow(clippy::too_many_arguments)]
fn run_in_worktree(
    issue: &IssueUrl,
    tickets: Vec<Ticket>,
    launch: &Git,
    selection: &Selection,
    base: &str,
    goal: Goal,
    base_fix: &mut BaseFix,
    logs: &Logs,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let branch = selection.branch();
    let worktree = match selection {
        Selection::Fresh { .. } => Worktree::create_fresh(launch, &issue.repo, branch, base)?,
        Selection::Continuation { .. } => {
            Worktree::continue_existing(launch, &issue.repo, branch, base)?
        }
    };
    if !tickets.is_empty() {
        return spec_run::run(
            issue, tickets, worktree, base, goal, base_fix, logs, parallel,
        );
    }
    let prompt = match selection {
        Selection::Fresh { .. } => prompt::fresh(issue, base, branch),
        Selection::Continuation { pr, .. } => {
            prompt::continuation(issue, base, branch, pr.as_ref().map(|pr| pr.url.as_str()))
        }
    };
    let mut log = logs.path("implement");
    let implemented = implement(
        issue, &worktree, base, &prompt, goal, base_fix, logs, &mut log,
    );
    match implemented {
        Ok(pr_url) => Ok(Reached {
            pr_url,
            goal,
            log: Some(log),
            ticket_lines: Vec::new(),
        }),
        Err(error) => Err(failed_run::fail(issue, worktree, base, &log, error)),
    }
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

/// The implement session given `prompt`, then [`deliver`] on the PR it
/// opened or updated. `log` is left at the most recent session's log.
#[allow(clippy::too_many_arguments)]
fn implement(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    prompt: &str,
    goal: Goal,
    base_fix: &mut BaseFix,
    logs: &Logs,
    log: &mut PathBuf,
) -> Result<String> {
    let plugin = Plugin::write()?;
    let sessions = Sessions {
        logs,
        worktree: worktree.path(),
        plugin_dir: plugin.path(),
    };
    let mut run_session = |kind: &str, prompt: &str| sessions.run(kind, prompt, log);

    run_session("implement", prompt)?;
    worktree.push()?;
    let pr = mark_pr_ready(issue, worktree.branch(), base)?;
    deliver(issue, worktree, base, &pr, goal, base_fix, &mut run_session)?;
    Ok(pr.url)
}

/// Take the ready PR `pr` for `issue`, from the branch checked out in
/// `worktree` into `base`, to `goal`: keep it mergeable and its CI green
/// through the Repair loop, starting each Repair through `run_session` and
/// its one Base fix, if any, through `base_fix`, and
/// for [`Goal::Merged`], Self-merge it. A merge that fails goes back round
/// the Repair loop and is tried again on the new head; if that round finds
/// nothing to fix, this fails with a `PolicyRefusal`.
pub fn deliver(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    pr: &PullRequest,
    goal: Goal,
    base_fix: &mut BaseFix,
    run_session: &mut impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    let branch = worktree.branch();
    let mut repair_loop = RepairLoop {
        issue,
        worktree,
        base,
        pr_url: &pr.url,
        goal,
        budgets: Budgets::default(),
        base_fix,
    };
    let mut watched = repair_loop.run(run_session)?;
    loop {
        ensure_pr_ready_and_mergeable(issue, branch)?;
        if interrupt::requested() {
            bail!("interrupted");
        }
        if goal == Goal::ReadyForReview {
            return Ok(());
        }
        progress::step(format_args!("merging the PR into {base}"));
        let Err(error) = github::merge(issue, branch, &watched) else {
            after_merge(issue, worktree, pr);
            return Ok(());
        };
        progress::step(format_args!("the merge failed: {error:#}"));
        match repair_loop.round_after_failed_merge(&watched, run_session)? {
            Round::NewHead(head) => watched = head,
            Round::NothingToFix => {
                // Only a PR still ready and mergeable is left ready.
                ensure_pr_ready_and_mergeable(issue, branch)?;
                return Err(error.context(PolicyRefusal));
            }
        }
    }
}

/// The Self-merge's steps after the merge: delete the Issue branch on origin,
/// and close the issue unless it is closed already. GitHub may close it too, a
/// moment after the merge, or may not, so thirdshift does not wait to see.
/// The merge can't be undone, so these never fail the Run, and an interrupt no
/// longer stops it: a step that fails is a warning naming the fix to make by
/// hand.
fn after_merge(issue: &IssueUrl, worktree: &Worktree, pr: &PullRequest) {
    let branch = worktree.branch();
    if let Err(error) = interrupt::retry_if_interrupted(|| worktree.delete_from_origin()) {
        progress::warn(
            &error,
            format_args!(
                "could not delete {branch} on origin, so delete it by hand: \
                 git push origin --delete {branch}"
            ),
        );
    }
    let comment = format!(
        "Closed by #{}, merged into {} by a thirdshift Merge run.",
        pr.number, pr.base
    );
    if let Err(error) = interrupt::retry_if_interrupted(|| close_unless_closed(issue, &comment)) {
        progress::warn(
            &error,
            format_args!(
                "could not close issue #{number}, so if it is still open, close it by hand: \
                 gh issue close {number} --repo {repo} --comment '{quoted}'",
                number = issue.number,
                repo = issue.repo_slug(),
                quoted = comment.replace('\'', r"'\''")
            ),
        );
    }
}

/// Close `issue` with `comment`, unless it is closed already.
fn close_unless_closed(issue: &IssueUrl, comment: &str) -> Result<()> {
    if !github::issue_is_open(issue)? {
        return Ok(());
    }
    progress::step(format_args!("closing issue #{}", issue.number));
    github::close_issue(issue, comment)
}

/// The most Repair sessions a Run starts, conflict, CI-fix and review
/// combined.
const MAX_REPAIRS: usize = 5;

/// The most times a Run goes round again because the Base branch moved while
/// CI ran or since a merge was tried, or, in a Merge run, because Foreign
/// commits arrived, whether or not the merge that follows needs a Repair. A
/// clean merge of the Base branch uses no Repair, so without this a busy Base
/// branch could keep a Run going forever.
const MAX_UPSTREAM_MOVES: usize = 5;

/// What a Run has spent of its Repair and upstream-move budgets, across every round
/// of the Repair loop, those after a failed merge included.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Budgets {
    repairs: usize,
    upstream_moves: usize,
}

impl Budgets {
    /// Counts the Repair about to start, as `repair-<n>`, or fails if it would
    /// be one too many.
    fn next_repair(&mut self, cause: &str) -> Result<String> {
        if self.repairs == MAX_REPAIRS {
            bail!("repairs exhausted: {cause}");
        }
        self.repairs += 1;
        progress::step(format_args!(
            "{cause}; starting Repair {} of {MAX_REPAIRS}",
            self.repairs
        ));
        Ok(format!("repair-{}", self.repairs))
    }

    /// Counts a round taken because `upstream` moved, saying `why`, or fails
    /// if it would be one too many.
    fn count_upstream_move(&mut self, upstream: &str, why: std::fmt::Arguments) -> Result<()> {
        if self.upstream_moves == MAX_UPSTREAM_MOVES {
            bail!("{upstream} kept moving: merged it again {MAX_UPSTREAM_MOVES} times");
        }
        self.upstream_moves += 1;
        progress::step(why);
        Ok(())
    }

    /// Counts a round taken because `origin/<base>` moved `when`.
    fn count_base_move(&mut self, base: &str, when: &str) -> Result<()> {
        self.count_upstream_move(
            &format!("origin/{base}"),
            format_args!("origin/{base} moved {when}; merging it again"),
        )
    }
}

/// What a round of the Repair loop after a failed merge came to.
enum Round {
    /// A head whose CI was found green or absent, to try the merge on.
    NewHead(String),
    /// The same head, with no Repair and no upstream move: a policy refusal.
    NothingToFix,
}

/// Keeps the PR mergeable and its CI green, within the Run's budgets.
struct RepairLoop<'a> {
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    pr_url: &'a str,
    goal: Goal,
    budgets: Budgets,
    /// The Run's one Base fix, kept apart from its budgets.
    base_fix: &'a mut BaseFix,
}

impl RepairLoop<'_> {
    /// In a Merge run, first take in any Foreign commits. Then merge the
    /// Base branch (never rebase), push, and watch CI on the head commit,
    /// starting a Repair session through `run_session` for a conflict or red
    /// CI and then going round again, since the Base branch may have moved
    /// meanwhile. Green or absent CI also goes round again if the Base branch
    /// moved while CI ran, or, in a Merge run, if Foreign commits arrived.
    /// A red check that also fails on the Base branch commit the head last
    /// merged in is an Inherited failure, which no Repair is started for: the
    /// CI-fix Repair is given the branch's own failures, and when there are
    /// none, the loop goes round again if the Base branch moved since, counted
    /// as a Base move, and otherwise once its Base fix has merged, or fails
    /// naming the checks and the Base branch commit: see [`BaseFix::fix`]. A
    /// Base fix's own Run sees no Inherited failures.
    /// Returns the head commit whose CI was last watched and found green or
    /// absent. Fails with a Declined CI fix if, after a CI-fix Repair and the
    /// Base branch merged again, the head is still the one whose CI failed,
    /// and once a Repair beyond `MAX_REPAIRS`, or a round
    /// beyond `MAX_UPSTREAM_MOVES`, would be needed.
    fn run(&mut self, run_session: &mut impl FnMut(&str, &str) -> Result<()>) -> Result<String> {
        let (issue, worktree, base, pr_url) = (self.issue, self.worktree, self.base, self.pr_url);
        let branch = worktree.branch();
        // The head whose red CI the last CI-fix Repair was given.
        let mut handed_to_repair = None;
        loop {
            if self.goal == Goal::Merged {
                self.take_in_foreign_commits(run_session)?;
            }
            if let Merge::Conflicted(pending) = worktree.merge_base_branch(base)? {
                let kind = self.budgets.next_repair("conflict")?;
                run_session(&kind, &prompt::conflict_repair(issue, base, branch, pr_url))?;
                worktree.ensure_merged(&pending)?;
                continue;
            }
            worktree.push()?;
            let head = worktree.head()?;
            if handed_to_repair.as_ref() == Some(&head) {
                bail!(
                    "CI red on {} and the Repair found nothing to fix on the branch",
                    ci::short(&head)
                );
            }
            let base_commit = worktree.merged_base_commit(base)?;
            let compared_with = self
                .base_fix
                .sees_inherited_failures()
                .then_some(base_commit.as_str());
            match ci::watch(issue, &head, compared_with)? {
                Ci::Absent | Ci::Passed => {
                    if worktree.base_branch_moved(base)? {
                        self.budgets.count_base_move(base, "while CI ran")?;
                    } else if self.goal == Goal::ReadyForReview
                        || worktree.new_commits_on_origin()?.is_empty()
                    {
                        return Ok(head);
                    }
                }
                Ci::Failed(failed) => {
                    if !failed.inherited.is_empty() {
                        progress::step(format_args!(
                            "Inherited failures (also failing on {base} at {}): {}",
                            ci::short(&base_commit),
                            ci::check_names(&failed.inherited)
                        ));
                    }
                    if failed.own.is_empty() {
                        // Someone may have fixed the Base branch since.
                        if worktree.base_branch_moved(base)? {
                            self.budgets.count_base_move(base, "while CI ran")?;
                            continue;
                        }
                        self.base_fix.fix(
                            worktree.launch(),
                            issue,
                            pr_url,
                            base,
                            &base_commit,
                            &failed.inherited,
                        )?;
                        continue;
                    }
                    let kind = self.budgets.next_repair("CI red")?;
                    run_session(
                        &kind,
                        &prompt::ci_fix_repair(issue, base, branch, pr_url, &failed),
                    )?;
                    handed_to_repair = Some(head);
                }
            }
        }
    }

    /// Fetch the Issue branch from origin and merge in any Foreign commits on
    /// it (never rebase), counting that as a round and handing a conflict to
    /// a conflict Repair. Then a review Repair reviews them from the head the
    /// Run last knew as its own, the local head before the merge. Goes round
    /// again until origin has nothing new, since more may land during either
    /// Repair, and the push after them would be rejected.
    fn take_in_foreign_commits(
        &mut self,
        run_session: &mut impl FnMut(&str, &str) -> Result<()>,
    ) -> Result<()> {
        let (issue, worktree, pr_url) = (self.issue, self.worktree, self.pr_url);
        let branch = worktree.branch();
        let upstream = worktree.upstream();
        loop {
            let foreign = worktree.new_commits_on_origin()?;
            if foreign.is_empty() {
                return Ok(());
            }
            let own_head = worktree.head()?;
            self.budgets.count_upstream_move(
                &upstream,
                format_args!("{upstream} has new commits; merging them in"),
            )?;
            for sha in &foreign {
                progress::step(format_args!("merging new commit {sha} from {upstream}"));
            }
            if let Merge::Conflicted(pending) = worktree.merge_new_commits()? {
                let kind = self
                    .budgets
                    .next_repair(&format!("conflict with new commits on {upstream}"))?;
                run_session(
                    &kind,
                    &prompt::conflict_repair(issue, branch, branch, pr_url),
                )?;
                worktree.ensure_merged(&pending)?;
            }
            let kind = self
                .budgets
                .next_repair(&format!("new commits on {upstream} to review"))?;
            run_session(
                &kind,
                &prompt::review_repair(issue, branch, pr_url, &own_head),
            )?;
        }
    }

    /// Go round again after a merge of `watched` failed, counting a Base
    /// branch that moved since as an upstream move. GitHub's error text is never
    /// consulted.
    fn round_after_failed_merge(
        &mut self,
        watched: &str,
        run_session: &mut impl FnMut(&str, &str) -> Result<()>,
    ) -> Result<Round> {
        let before = self.budgets;
        if self.worktree.base_branch_moved(self.base)? {
            self.budgets
                .count_base_move(self.base, "since the merge was tried")?;
        }
        let head = self.run(run_session)?;
        Ok(if head == watched && self.budgets == before {
            Round::NothingToFix
        } else {
            Round::NewHead(head)
        })
    }
}

/// Mark the PR whose head is `branch` ready for review, failing unless it
/// exists, is open and targets `base`.
pub fn mark_pr_ready(issue: &IssueUrl, branch: &str, base: &str) -> Result<PullRequest> {
    progress::step("checking the PR");
    let pr = open_pr(issue, branch)?;
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    if pr.is_draft {
        github::mark_ready(issue, branch)?;
    }
    Ok(pr)
}

/// The PR whose head is `branch`, failing unless it exists and is open.
fn open_pr(issue: &IssueUrl, branch: &str) -> Result<PullRequest> {
    let pr = github::pull_request_for(issue, branch)?.context("no PR found")?;
    if !pr.is_open() {
        bail!("PR {} is {}, not open", pr.url, pr.state);
    }
    Ok(pr)
}

/// Fail unless the PR for `branch` is still open, ready for review, and
/// mergeable, waiting up to the grace period for GitHub to work out the last.
fn ensure_pr_ready_and_mergeable(issue: &IssueUrl, branch: &str) -> Result<()> {
    progress::step("checking the PR is open, ready and mergeable");
    let pr = open_pr(issue, branch)?;
    if pr.is_draft {
        bail!("PR {} is a draft", pr.url);
    }
    let mergeable = poll::within(poll::grace_period(), || {
        Ok(Some(github::mergeable(issue, branch)?).filter(|m| *m != Mergeable::Unknown))
    })?;
    match mergeable {
        Some(Mergeable::Yes) => Ok(()),
        Some(Mergeable::No) => bail!("PR {} is not mergeable", pr.url),
        _ => bail!(
            "GitHub has not worked out whether PR {} is mergeable",
            pr.url
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_of_every_kind_share_one_cap_and_the_one_too_many_names_its_cause() {
        let mut budgets = Budgets::default();
        for n in 1..=MAX_REPAIRS {
            let cause = if n % 2 == 0 { "conflict" } else { "CI red" };
            assert_eq!(budgets.next_repair(cause).unwrap(), format!("repair-{n}"));
        }

        for cause in ["new commits on origin/issue-7 to review", "conflict"] {
            let error = budgets.next_repair(cause).unwrap_err();
            assert_eq!(error.to_string(), format!("repairs exhausted: {cause}"));
        }
        assert_eq!(budgets.repairs, MAX_REPAIRS);
    }

    #[test]
    fn upstream_moves_of_the_base_or_the_issue_branch_share_one_budget() {
        let mut budgets = Budgets::default();
        for _ in 1..MAX_UPSTREAM_MOVES {
            budgets.count_base_move("main", "while CI ran").unwrap();
        }
        budgets
            .count_upstream_move("origin/issue-7", format_args!("new commits"))
            .unwrap();

        let error = budgets.count_base_move("main", "while CI ran").unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/main kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
        let error = budgets
            .count_upstream_move("origin/issue-7", format_args!("new commits"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/issue-7 kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
    }

    #[test]
    fn the_repair_cap_and_the_upstream_move_budget_are_spent_apart() {
        let mut budgets = Budgets::default();
        for _ in 0..MAX_REPAIRS {
            budgets.next_repair("CI red").unwrap();
        }

        for _ in 0..MAX_UPSTREAM_MOVES {
            budgets.count_base_move("main", "while CI ran").unwrap();
        }
        assert_eq!(
            budgets,
            Budgets {
                repairs: MAX_REPAIRS,
                upstream_moves: MAX_UPSTREAM_MOVES
            }
        );
    }
}
