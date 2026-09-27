//! One Run: from an Issue URL to a checked PR, or to a Failed run.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use crate::branch::{self, Selection};
use crate::ci::{self, Ci};
use crate::failed_run::{self, FailedRun};
use crate::git::Git;
use crate::github::{self, Mergeable, PrState, PullRequest};
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
/// `log` is left at the most recent session's log.
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

    let watched = repair_loop(issue, worktree, base, &pr.url, &mut run_session)?;
    ensure_pr_ready_and_mergeable(issue, branch)?;
    if interrupt::requested() {
        bail!("interrupted");
    }
    if goal == Goal::Merged {
        progress::step(format_args!("merging the PR into {base}"));
        merge(issue, branch, &watched)?;
        after_merge(issue, worktree, base, &pr);
    }
    Ok(pr.url)
}

/// Merge the PR for `branch` on `head`. A merge that reports failure but went
/// through anyway, e.g. because `gh` was interrupted after GitHub merged, is
/// a merge: whether the PR merged decides the Run's outcome.
fn merge(issue: &IssueUrl, branch: &str, head: &str) -> Result<()> {
    let error = match github::merge(issue, branch, head) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    match github::pull_request_for(issue, branch) {
        Ok(Some(pr)) if pr.state == PrState::Merged => Ok(()),
        _ => Err(error),
    }
}

/// The Self-merge's steps after the merge: delete the Issue branch on origin,
/// and close the issue if the merge did not. The merge can't be undone, so
/// these never fail the Run, and an interrupt no longer stops it: a step that
/// fails is a warning naming the fix to make by hand.
fn after_merge(issue: &IssueUrl, worktree: &Worktree, base: &str, pr: &PullRequest) {
    let branch = worktree.branch();
    if let Err(error) = worktree.delete_from_origin() {
        warn(
            &error,
            format_args!(
                "could not delete {branch} on origin, so delete it by hand: \
                 git push origin --delete {branch}"
            ),
        );
    }
    let comment = format!(
        "Closed by #{}, merged into {base} by a thirdshift Merge run.",
        pr.number
    );
    let closed = github::issue_is_open(issue).and_then(|open| {
        if open {
            progress::step(format_args!("closing issue #{}", issue.number));
            github::close_issue(issue, &comment)?;
        }
        Ok(())
    });
    if let Err(error) = closed {
        warn(
            &error,
            format_args!(
                "could not close issue #{number}, so close it by hand: \
                 gh issue close {number} --repo {repo} --comment '{comment}'",
                number = issue.number,
                repo = issue.repo_slug()
            ),
        );
    }
}

/// Report `error`, then a warning saying what to do about it by hand.
fn warn(error: &anyhow::Error, warning: std::fmt::Arguments) {
    progress::step(format_args!("{error:#}"));
    progress::step(format_args!("warning: {warning}"));
}

/// The most Repair sessions a Run starts, conflict and CI-fix combined.
const MAX_REPAIRS: usize = 5;

/// The most times a Run goes round again because the Base branch moved while
/// CI ran, whether or not the merge that follows needs a Repair. A clean merge
/// uses no Repair, so without this a busy Base branch could keep a Run going
/// forever.
const MAX_BASE_MOVES: usize = 5;

/// Keep the PR mergeable and its CI green: merge the Base branch (never
/// rebase), push, and watch CI on the head commit, starting a Repair session
/// through `run_session` for a conflict or red CI and then going round again,
/// since the Base branch may have moved meanwhile. Green or absent CI also
/// goes round again if the Base branch moved while CI ran. Returns the head
/// commit whose CI was last watched and found green or absent. Fails once a
/// Repair beyond `MAX_REPAIRS`, or a round beyond `MAX_BASE_MOVES`, would be
/// needed.
fn repair_loop(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    pr_url: &str,
    run_session: &mut impl FnMut(&str, &str) -> Result<()>,
) -> Result<String> {
    let branch = worktree.branch();
    let mut repairs = 0;
    let mut base_moves = 0;
    // Counts the Repair about to start, as `repair-<n>`, or fails if it would
    // be one too many.
    let mut next_repair = |cause: &str| -> Result<String> {
        if repairs == MAX_REPAIRS {
            bail!("repairs exhausted: {cause}");
        }
        repairs += 1;
        progress::step(format_args!(
            "{cause}; starting Repair {repairs} of {MAX_REPAIRS}"
        ));
        Ok(format!("repair-{repairs}"))
    };
    loop {
        if worktree.merge_base_branch(base)? == Merge::Conflicted {
            let kind = next_repair("conflict")?;
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
                if base_moves == MAX_BASE_MOVES {
                    bail!(
                        "origin/{base} kept moving while CI ran: merged it again {MAX_BASE_MOVES} times"
                    );
                }
                base_moves += 1;
                progress::step(format_args!(
                    "origin/{base} moved while CI ran; merging it again"
                ));
            }
            Ci::Failed(failed) => {
                let kind = next_repair("CI red")?;
                run_session(
                    &kind,
                    &prompt::ci_fix_repair(issue, base, branch, pr_url, &failed),
                )?;
                worktree.push()?;
            }
        }
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
