//! One Run: from an Issue URL to a checked PR, or to a Failed run.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use crate::branch::{self, Selection};
use crate::ci::{self, Ci};
use crate::failed_run::{self, FailedRun, PolicyRefusal};
use crate::git::Git;
use crate::github::{self, Mergeable, PullRequest};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::poll;
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::session::{self, Sessions};
use crate::worktree::{Merge, Worktree};

/// Where a Run takes its PR: ready for review, or, in a Merge run, merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    ReadyForReview,
    Merged,
}

impl Goal {
    /// What became of the PR once the Run reached this goal, as in
    /// "PR <url> is merged".
    pub fn outcome(self) -> &'static str {
        match self {
            Goal::ReadyForReview => "is ready for review",
            Goal::Merged => "is merged",
        }
    }
}

/// Take `issue` to a ready PR, or in a Merge run a merged one, and return the
/// PR's URL. Any failure after the worktree exists, including a merge that
/// fails, goes through the Failed run path. The worktree, the local
/// Issue branch and the plugin directory are gone when this returns, except
/// that a Failed run whose work did not reach origin keeps the worktree and
/// branch.
pub fn run(issue: &IssueUrl, goal: Goal) -> Result<String, FailedRun> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    preflight::check(&launch, issue)?;
    let checked_out = launch
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok();
    let selection = branch::select(&launch, issue)?;
    let branch = selection.branch().to_string();
    let base = selection.base_branch(checked_out.as_deref())?;
    preflight::check_base_branch(&launch, &base)?;

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let (worktree, prompt) = match &selection {
        Selection::Fresh { .. } => (
            Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?,
            prompt::fresh(issue, &base, &branch),
        ),
        Selection::Continuation { pr, .. } => (
            Worktree::continue_existing(&launch, &issue.repo, &branch, &base)?,
            prompt::continuation(issue, &base, &branch, pr.as_ref().map(|pr| pr.url.as_str())),
        ),
    };
    let mut log = session::log_path(issue, &timestamp, "implement")?;
    implement(issue, &worktree, &base, &prompt, goal, &timestamp, &mut log)
        .map_err(|error| failed_run::fail(issue, worktree, &base, &log, error))
}

/// The implement session given `prompt`, the checks on the PR it opened or
/// updated, keeping that PR mergeable and, in a Merge run, the Self-merge.
/// A merge that fails goes back round the Repair loop and is tried again on
/// the new head; if that round finds nothing to fix, the Run fails with a
/// `PolicyRefusal`. `log` is left at the most recent session's log.
fn implement(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    prompt: &str,
    goal: Goal,
    timestamp: &str,
    log: &mut PathBuf,
) -> Result<String> {
    let branch = worktree.branch();
    let plugin = Plugin::write()?;
    let sessions = Sessions {
        issue,
        timestamp,
        worktree: worktree.path(),
        plugin_dir: plugin.path(),
    };
    let mut run_session = |kind: &str, prompt: &str| sessions.run(kind, prompt, log);

    run_session("implement", prompt)?;
    worktree.push()?;

    progress::step("checking the PR");
    let pr = open_pr(issue, branch)?;
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    if pr.is_draft {
        github::mark_ready(issue, branch)?;
    }

    let mut repair_loop = RepairLoop {
        issue,
        worktree,
        base,
        pr_url: &pr.url,
        budgets: Budgets::default(),
    };
    let mut watched = repair_loop.run(&mut run_session)?;
    loop {
        ensure_pr_ready_and_mergeable(issue, branch)?;
        if interrupt::requested() {
            bail!("interrupted");
        }
        if goal == Goal::ReadyForReview {
            return Ok(pr.url);
        }
        progress::step(format_args!("merging the PR into {base}"));
        let Err(error) = github::merge(issue, branch, &watched) else {
            after_merge(issue, worktree, &pr);
            return Ok(pr.url);
        };
        progress::step(format_args!("the merge failed: {error:#}"));
        match repair_loop.round_after_failed_merge(&watched, &mut run_session)? {
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
/// and close the issue if the merge does not. The merge can't be undone, so
/// these never fail the Run, and an interrupt no longer stops it: a step that
/// fails is a warning naming the fix to make by hand.
fn after_merge(issue: &IssueUrl, worktree: &Worktree, pr: &PullRequest) {
    let branch = worktree.branch();
    if let Err(error) = retry_if_interrupted(|| worktree.delete_from_origin()) {
        warn(
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
    if let Err(error) = retry_if_interrupted(|| close_unless_merge_does(issue, branch, &comment)) {
        warn(
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

/// Close `issue` with `comment`, unless it is closed already or merging the
/// PR for `branch` closes it.
fn close_unless_merge_does(issue: &IssueUrl, branch: &str, comment: &str) -> Result<()> {
    if github::merge_closes_issue(issue, branch)? || !github::issue_is_open(issue)? {
        return Ok(());
    }
    progress::step(format_args!("closing issue #{}", issue.number));
    github::close_issue(issue, comment)
}

/// Run `step`, and once more if it failed with the Run interrupted: Ctrl-C in
/// a terminal also kills the git or gh the step was running.
fn retry_if_interrupted(step: impl Fn() -> Result<()>) -> Result<()> {
    step().or_else(|error| {
        if interrupt::requested() {
            step()
        } else {
            Err(error)
        }
    })
}

/// Report `error`, then a warning saying what to do about it by hand.
fn warn(error: &anyhow::Error, warning: std::fmt::Arguments) {
    progress::step(format_args!("{error:#}"));
    progress::step(format_args!("warning: {warning}"));
}

/// The most Repair sessions a Run starts, conflict and CI-fix combined.
const MAX_REPAIRS: usize = 5;

/// The most times a Run goes round again because the Base branch moved while
/// CI ran or since a merge was tried, whether or not the merge that follows
/// needs a Repair. A clean merge uses no Repair, so without this a busy Base
/// branch could keep a Run going forever.
const MAX_BASE_MOVES: usize = 5;

/// What a Run has spent of its Repair and base-move budgets, across every
/// round of the Repair loop, those after a failed merge included.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Budgets {
    repairs: usize,
    base_moves: usize,
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

    /// Counts a round taken because `origin/<base>` moved `when`, or fails if
    /// it would be one too many.
    fn count_base_move(&mut self, base: &str, when: &str) -> Result<()> {
        if self.base_moves == MAX_BASE_MOVES {
            bail!("origin/{base} kept moving: merged it again {MAX_BASE_MOVES} times");
        }
        self.base_moves += 1;
        progress::step(format_args!("origin/{base} moved {when}; merging it again"));
        Ok(())
    }
}

/// What a round of the Repair loop after a failed merge came to.
enum Round {
    /// A head whose CI was found green or absent, to try the merge on.
    NewHead(String),
    /// The same head, with no Repair and no base move: a policy refusal.
    NothingToFix,
}

/// Keeps the PR mergeable and its CI green, within the Run's budgets.
struct RepairLoop<'a> {
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    pr_url: &'a str,
    budgets: Budgets,
}

impl RepairLoop<'_> {
    /// Merge the Base branch (never rebase), push, and watch CI on the head
    /// commit, starting a Repair session through `run_session` for a conflict
    /// or red CI and then going round again, since the Base branch may have
    /// moved meanwhile. Green or absent CI also goes round again if the Base
    /// branch moved while CI ran. Returns the head commit whose CI was last
    /// watched and found green or absent. Fails once a Repair beyond
    /// `MAX_REPAIRS`, or a round beyond `MAX_BASE_MOVES`, would be needed.
    fn run(&mut self, run_session: &mut impl FnMut(&str, &str) -> Result<()>) -> Result<String> {
        let (issue, worktree, base, pr_url) = (self.issue, self.worktree, self.base, self.pr_url);
        let branch = worktree.branch();
        loop {
            if worktree.merge_base_branch(base)? == Merge::Conflicted {
                let kind = self.budgets.next_repair("conflict")?;
                run_session(&kind, &prompt::conflict_repair(issue, base, branch, pr_url))?;
                worktree.ensure_base_branch_merged(base)?;
                worktree.push()?;
                continue;
            }
            worktree.push()?;
            let head = worktree.head()?;
            match ci::watch(issue, &head)? {
                Ci::Absent | Ci::Passed => {
                    if !worktree.base_branch_moved(base)? {
                        return Ok(head);
                    }
                    self.budgets.count_base_move(base, "while CI ran")?;
                }
                Ci::Failed(failed) => {
                    let kind = self.budgets.next_repair("CI red")?;
                    run_session(
                        &kind,
                        &prompt::ci_fix_repair(issue, base, branch, pr_url, &failed),
                    )?;
                    worktree.push()?;
                }
            }
        }
    }

    /// Go round again after a merge of `watched` failed, counting a Base
    /// branch that moved since as a base move. GitHub's error text is never
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
