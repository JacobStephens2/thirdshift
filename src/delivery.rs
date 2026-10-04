//! Delivery: what a Run, or a Spec run for its Spec PR, does from its
//! worktree once its work begins. The opening session, the push after it,
//! marking the pull request ready, the Repair loop that keeps it mergeable
//! and green, the Self-merge in a Merge run, and the Failed run path when any
//! of these fails.

use anyhow::{Context, Result, bail};

mod repair_loop;

use crate::base_fix::BaseFix;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{self, FailedRun};
use crate::github::{self, Mergeable, PullRequest};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::poll;
use crate::progress;
use crate::prompt;
use crate::run::{Goal, Reached};
use crate::session::{Logs, Sessions};
use crate::worktree::{Merge, PendingMerge, Worktree};

use repair_loop::{Repair, Upstream};

/// A Delivery of the pull request for `issue`, the Run's issue or the Spec,
/// into the Base branch `base`, to `goal`.
pub struct Delivery<'a> {
    pub issue: &'a IssueUrl,
    pub base: &'a str,
    pub goal: Goal,
    /// The one Base fix the Delivery may start, or wait on, when its only red
    /// checks are Inherited failures.
    pub base_fix: &'a mut BaseFix,
    /// Where its Session logs go.
    pub logs: &'a Logs,
}

/// The session a Delivery opens with: the implement session, or the Spec
/// review.
pub struct Opening<'a> {
    /// The session's kind, in its progress lines and log name.
    pub kind: &'a str,
    pub prompt: String,
    /// Whether the branch is first fast-forwarded to its head on origin, as
    /// the Spec branch is to the Tickets landed there.
    pub catch_up_from_origin: bool,
}

impl Delivery<'_> {
    /// Take the pull request from the branch checked out in `worktree` to the
    /// goal. With `opening.catch_up_from_origin`, first fast-forward the
    /// branch to origin. Then run the opening session, push the branch, for
    /// any commit the session left unpushed, run `before_ready`, and mark the
    /// pull request ready, failing unless it exists, is open and targets the
    /// Base branch. Then keep it mergeable and its CI green through the
    /// Repair loop, and for [`Goal::Merged`], Self-merge it. A merge that
    /// fails goes back round the Repair loop and is tried again on the new
    /// head; if that round finds nothing to fix, this fails with a
    /// `PolicyRefusal`. Any failure goes through the Failed run path, which
    /// keeps the worktree if its work did not reach origin; otherwise it is
    /// cleaned up when this returns.
    pub fn deliver(
        mut self,
        worktree: Worktree,
        opening: Opening,
        before_ready: impl FnOnce() -> Result<()>,
    ) -> Result<Reached, FailedRun> {
        let caught_up = if opening.catch_up_from_origin {
            worktree.fast_forward_to_origin()
        } else {
            Ok(())
        };
        let (delivered, log) = match caught_up {
            Ok(()) => Sessions::within(self.logs, worktree.path(), |sessions| {
                sessions.run(opening.kind, &opening.prompt)?;
                worktree.push()?;
                before_ready()?;
                let pr = mark_pr_ready(self.issue, worktree.branch(), self.base)?;
                self.take_to_goal(sessions, &worktree, &pr)?;
                Ok(pr.url)
            }),
            Err(error) => (Err(error), None),
        };
        match delivered {
            Ok(pr_url) => Ok(Reached {
                pr_url,
                goal: self.goal,
                log,
                ticket_lines: Vec::new(),
            }),
            Err(error) => Err(failed_run::fail(
                self.issue, worktree, self.base, log, error,
            )),
        }
    }

    /// Take the ready PR `pr`, from the branch checked out in `worktree`, to
    /// the goal, as [`Delivery::deliver`] says, starting each Repair through
    /// `sessions`, then for [`Goal::Merged`], take the Self-merge's steps
    /// after the merge.
    fn take_to_goal(
        &mut self,
        sessions: &Sessions,
        worktree: &Worktree,
        pr: &PullRequest,
    ) -> Result<()> {
        let mut outside = RunOutside {
            issue: self.issue,
            worktree,
            base: self.base,
            pr_url: &pr.url,
            base_fix: self.base_fix,
            sessions,
        };
        repair_loop::take_to_goal(&mut outside, self.base, self.goal)?;
        if self.goal == Goal::Merged {
            after_merge(self.issue, worktree, pr);
        }
        Ok(())
    }
}

/// What a Run's Repair loop reaches outside itself: the Run's worktree, its
/// Repair sessions, its Base fix, its issue and its pull request.
struct RunOutside<'a> {
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    pr_url: &'a str,
    base_fix: &'a mut BaseFix,
    sessions: &'a Sessions<'a>,
}

impl repair_loop::Outside for RunOutside<'_> {
    type Pending = PendingMerge;

    fn merge_base_branch(&mut self) -> Result<Option<PendingMerge>> {
        Ok(pending(self.worktree.merge_base_branch(self.base)?))
    }

    fn merge_new_commits(&mut self) -> Result<Option<PendingMerge>> {
        Ok(pending(self.worktree.merge_new_commits()?))
    }

    fn ensure_merged(&mut self, pending: &PendingMerge) -> Result<()> {
        self.worktree.ensure_merged(pending)
    }

    fn push(&mut self) -> Result<()> {
        self.worktree.push()
    }

    fn head(&mut self) -> Result<String> {
        self.worktree.head()
    }

    fn merged_base_commit(&mut self) -> Result<String> {
        self.worktree.merged_base_commit(self.base)
    }

    fn base_branch_moved(&mut self) -> Result<bool> {
        self.worktree.base_branch_moved(self.base)
    }

    fn new_commits_on_origin(&mut self) -> Result<Vec<String>> {
        self.worktree.new_commits_on_origin()
    }

    fn upstream(&mut self) -> String {
        self.worktree.upstream()
    }

    fn watch(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci> {
        ci::watch(self.issue, head, base_commit)
    }

    fn rerun(
        &mut self,
        head: &str,
        base_commit: Option<&str>,
        failed: &FailedChecks,
    ) -> Result<Option<Ci>> {
        ci::rerun(self.issue, head, base_commit, failed)
    }

    fn repair(&mut self, kind: &str, repair: Repair) -> Result<()> {
        let (issue, base, pr_url) = (self.issue, self.base, self.pr_url);
        let branch = self.worktree.branch();
        let prompt = match repair {
            Repair::Conflict(Upstream::BaseBranch) => {
                prompt::conflict_repair(issue, base, branch, pr_url)
            }
            Repair::Conflict(Upstream::IssueBranch) => {
                prompt::conflict_repair(issue, branch, branch, pr_url)
            }
            Repair::CiFix(failed) => prompt::ci_fix_repair(issue, base, branch, pr_url, failed),
            Repair::Review { from } => prompt::review_repair(issue, branch, pr_url, from),
        };
        self.sessions.run(kind, &prompt)
    }

    fn sees_inherited_failures(&mut self) -> bool {
        self.base_fix.sees_inherited_failures()
    }

    fn base_fix(&mut self, base_commit: &str, failed: &FailedChecks) -> Result<()> {
        self.base_fix.fix(
            self.worktree.launch(),
            self.issue,
            self.pr_url,
            self.base,
            base_commit,
            failed,
        )
    }

    fn ensure_pr_ready_and_mergeable(&mut self) -> Result<()> {
        ensure_pr_ready_and_mergeable(self.issue, self.worktree.branch())
    }

    fn merge(&mut self, head: &str) -> Result<()> {
        github::merge(self.issue, self.worktree.branch(), head)
    }

    fn interrupt_requested(&mut self) -> bool {
        interrupt::requested()
    }

    fn progress(&mut self, line: String) {
        progress::step(line);
    }
}

/// The merge `merge` left pending for a conflict Repair, if it conflicted.
fn pending(merge: Merge) -> Option<PendingMerge> {
    match merge {
        Merge::Clean => None,
        Merge::Conflicted(pending) => Some(pending),
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

/// Mark the PR whose head is `branch` ready for review, failing unless it
/// exists, is open and targets `base`.
fn mark_pr_ready(issue: &IssueUrl, branch: &str, base: &str) -> Result<PullRequest> {
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
