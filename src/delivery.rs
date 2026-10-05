//! Delivery: what a Run, or a Spec run for its Spec PR, does from its
//! worktree once its work begins. The opening session, the push after it,
//! marking the pull request ready, the Repair loop that keeps it mergeable
//! and green, the Self-merge in a Merge run and its steps after the merge,
//! and the Failed run path when any of these fails.
//!
//! Its steps reach the worktree, the sessions, the Repair loop, GitHub, the
//! interrupt and the progress lines only through [`Outside`], and the Failed
//! run path, which runs once the sessions have ended, only through
//! [`FailedOutside`]: [`InWorktree`] and [`OnFailure`] do each for real;
//! `Scripted`, in tests, from a script, recording each call.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{SecondsFormat, Utc};

mod repair_loop;

use crate::base_fix::BaseFix;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{FailedRun, PolicyRefusal, interrupted_or};
use crate::github::{self, Mergeable, PullRequest};
use crate::host;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::poll;
use crate::progress;
use crate::prompt;
use crate::run::{Goal, Reached};
use crate::run_ending::Cause;
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
        self,
        worktree: Worktree,
        opening: Opening,
        before_ready: impl FnOnce() -> Result<()>,
    ) -> Result<Reached, FailedRun> {
        let route = Route {
            issue: self.issue,
            base: self.base,
            branch: worktree.branch(),
            goal: self.goal,
        };
        let (delivered, log) = Sessions::within(self.logs, worktree.path(), |sessions| {
            let mut outside = InWorktree {
                issue: self.issue,
                worktree: &worktree,
                base: self.base,
                base_fix: self.base_fix,
                sessions,
            };
            route.steps(&mut outside, &opening, |_| before_ready())
        });
        match delivered {
            Ok(pr_url) => Ok(Reached {
                pr_url,
                goal: self.goal,
                log,
                ticket_lines: Vec::new(),
            }),
            Err(error) => {
                let mut outside = OnFailure {
                    issue: self.issue,
                    base: self.base,
                    worktree,
                };
                Err(fail(&mut outside, log, error))
            }
        }
    }
}

/// The steps of a Delivery of the pull request for `issue`, from `branch`
/// into the Base branch `base`, to `goal`.
struct Route<'a> {
    issue: &'a IssueUrl,
    base: &'a str,
    branch: &'a str,
    goal: Goal,
}

impl Route<'_> {
    /// [`Delivery::deliver`]'s steps, through `outside`, in order: catch up
    /// from origin if `opening` says to, the opening session, the push,
    /// `before_ready`, marking the pull request ready, taking it to the goal
    /// through the Repair loop, then for [`Goal::Merged`], the steps after
    /// the merge. Returns the pull request's URL.
    fn steps<O: Outside>(
        &self,
        outside: &mut O,
        opening: &Opening,
        before_ready: impl FnOnce(&mut O) -> Result<()>,
    ) -> Result<String> {
        if opening.catch_up_from_origin {
            outside.catch_up()?;
        }
        outside.session(opening.kind, &opening.prompt)?;
        outside.push()?;
        before_ready(outside)?;
        let pr = self.mark_pr_ready(outside)?;
        outside.take_to_goal(&pr.url, self.goal)?;
        if self.goal == Goal::Merged {
            self.after_merge(outside, &pr);
        }
        Ok(pr.url)
    }

    /// Mark the PR for the Issue branch ready for review, failing unless it
    /// exists, is open and targets the Base branch.
    fn mark_pr_ready(&self, outside: &mut impl Outside) -> Result<PullRequest> {
        outside.step("checking the PR".to_string());
        let pr = open(outside.pull_request()?)?;
        if pr.base != self.base {
            bail!("PR targets {}, not {}", pr.base, self.base);
        }
        if pr.is_draft {
            outside.mark_ready()?;
        }
        Ok(pr)
    }

    /// The Self-merge's steps after the merge of `pr`: delete the Issue
    /// branch on origin, and close the issue unless it is closed already.
    /// GitHub may close it too, a moment after the merge, or may not, so
    /// thirdshift does not wait to see. The merge can't be undone, so these
    /// never fail the Run, and an interrupt no longer stops it: a step that
    /// fails is tried once more if an interrupt was requested, then is a
    /// warning naming the fix to make by hand.
    fn after_merge(&self, outside: &mut impl Outside, pr: &PullRequest) {
        let branch = self.branch;
        if let Err(error) = retry_if_interrupted(outside, |outside| outside.delete_branch()) {
            outside.warn(
                &error,
                format!(
                    "could not delete {branch} on origin, so delete it by hand: \
                     git push origin --delete {branch}"
                ),
            );
        }
        let comment = format!(
            "Closed by #{}, merged into {} by a thirdshift Merge run.",
            pr.number, pr.base
        );
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            self.close_unless_closed(outside, &comment)
        }) {
            let issue = self.issue;
            outside.warn(
                &error,
                format!(
                    "could not close issue #{number}, so if it is still open, close it by hand: \
                     gh issue close {number} --repo {repo} --comment '{quoted}'",
                    number = issue.number,
                    repo = issue.repo_slug(),
                    quoted = comment.replace('\'', r"'\''")
                ),
            );
        }
    }

    /// Close the issue with `comment`, unless it is closed already.
    fn close_unless_closed(&self, outside: &mut impl Outside, comment: &str) -> Result<()> {
        if !outside.issue_is_open()? {
            return Ok(());
        }
        outside.step(format!("closing issue #{}", self.issue.number));
        outside.close_issue(comment)
    }
}

/// Run `step` through `outside`, and once more if it failed with an
/// interrupt requested: Ctrl-C in a terminal also kills the git or gh the
/// step was running.
fn retry_if_interrupted<O: Outside>(
    outside: &mut O,
    step: impl Fn(&mut O) -> Result<()>,
) -> Result<()> {
    match step(outside) {
        Err(_) if outside.interrupted() => step(outside),
        done => done,
    }
}

/// `pr`, failing unless it exists and is open.
fn open(pr: Option<PullRequest>) -> Result<PullRequest> {
    let pr = pr.context("no PR found")?;
    if !pr.is_open() {
        bail!("PR {} is {}, not open", pr.url, pr.state);
    }
    Ok(pr)
}

/// Take the Run down the Failed run path through `outside`, once its
/// sessions have ended with `error`: commit and push the work, and send an
/// open PR back to draft. An interrupt, if one was requested, is the error
/// instead: it can surface as some other error, such as a killed git. A
/// `PolicyRefusal` neither pushes nor converts, so the PR stays ready on the
/// head whose CI was watched. Problems along the way are reported, not
/// raised, so the error is what the Run fails with. The worktree is kept if
/// its work may not have reached origin. `log` is the most recent Session
/// log, if a session created one.
fn fail(outside: &mut impl FailedOutside, log: Option<PathBuf>, error: anyhow::Error) -> FailedRun {
    let interrupted = outside.interrupted();
    let error = interrupted_or(error, interrupted);
    // The cause as stderr gives it, down to what a session left running, cut
    // to its first line: the reason goes in the failure commit's subject.
    let reason = Cause::of(&error).first_line().to_string();
    // Everything is already on origin: the Repair loop pushed the head.
    let keep_ready = error.is::<PolicyRefusal>();
    let pushed = if keep_ready {
        Ok(())
    } else {
        outside.commit_and_push(&reason)
    };
    if let Err(problem) = &pushed {
        outside.step(format!(
            "could not push the failed run's work, so it may exist only locally: {problem:#}"
        ));
    }
    let pr_url = match open_pr_url(outside, keep_ready) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            outside.step(format!("could not convert the PR to a draft: {problem:#}"));
            None
        }
    };
    if pushed.is_err() {
        outside.keep();
    }
    FailedRun {
        error,
        pr_url,
        log,
        interrupted,
        ticket_lines: Vec::new(),
    }
}

/// The URL of the open PR for the Issue branch, if there is one, converted
/// to a draft unless `keep_ready`.
fn open_pr_url(outside: &mut impl FailedOutside, keep_ready: bool) -> Result<Option<String>> {
    let Some(pr) = outside.pull_request()?.filter(|pr| pr.is_open()) else {
        return Ok(None);
    };
    if !pr.is_draft && !keep_ready {
        outside.convert_to_draft()?;
    }
    Ok(Some(pr.url))
}

/// What a Delivery's steps do or read outside the Delivery, within its
/// sessions' scope.
trait Outside {
    /// Fast-forward the Issue branch to its head on origin.
    fn catch_up(&mut self) -> Result<()>;
    /// Run the session `kind` given `prompt`.
    fn session(&mut self, kind: &str, prompt: &str) -> Result<()>;
    /// Push the Issue branch.
    fn push(&mut self) -> Result<()>;
    /// The pull request for the Issue branch, if it has one.
    fn pull_request(&mut self) -> Result<Option<PullRequest>>;
    /// Mark the pull request ready for review.
    fn mark_ready(&mut self) -> Result<()>;
    /// Take the ready pull request at `pr_url` to `goal` through the Repair
    /// loop.
    fn take_to_goal(&mut self, pr_url: &str, goal: Goal) -> Result<()>;
    /// Delete the Issue branch on origin.
    fn delete_branch(&mut self) -> Result<()>;
    /// Whether the issue is open.
    fn issue_is_open(&mut self) -> Result<bool>;
    /// Close the issue with `comment`.
    fn close_issue(&mut self, comment: &str) -> Result<()>;
    /// Whether an interrupt was requested.
    fn interrupted(&mut self) -> bool;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
    /// Hand on `error`, then the warning `warning` saying what to do about it
    /// by hand.
    fn warn(&mut self, error: &anyhow::Error, warning: String);
}

/// What the Failed run path does or reads outside the Delivery, once its
/// sessions have ended.
trait FailedOutside {
    /// Commit the work as the failure commit for `reason`, and push it.
    fn commit_and_push(&mut self, reason: &str) -> Result<()>;
    /// The pull request for the Issue branch, if it has one.
    fn pull_request(&mut self) -> Result<Option<PullRequest>>;
    /// Convert the pull request back to a draft.
    fn convert_to_draft(&mut self) -> Result<()>;
    /// Keep the worktree and the local Issue branch, for work that may exist
    /// nowhere else.
    fn keep(&mut self);
    /// Whether an interrupt was requested.
    fn interrupted(&mut self) -> bool;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// A Delivery's worktree, its sessions, its Base fix, and its issue and pull
/// request on GitHub.
struct InWorktree<'a> {
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    base_fix: &'a mut BaseFix,
    sessions: &'a Sessions<'a>,
}

impl Outside for InWorktree<'_> {
    fn catch_up(&mut self) -> Result<()> {
        self.worktree.fast_forward_to_origin()
    }

    fn session(&mut self, kind: &str, prompt: &str) -> Result<()> {
        self.sessions.run(kind, prompt)
    }

    fn push(&mut self) -> Result<()> {
        self.worktree.push()
    }

    fn pull_request(&mut self) -> Result<Option<PullRequest>> {
        github::pull_request_for(self.issue, self.worktree.branch())
    }

    fn mark_ready(&mut self) -> Result<()> {
        github::mark_ready(self.issue, self.worktree.branch())
    }

    fn take_to_goal(&mut self, pr_url: &str, goal: Goal) -> Result<()> {
        let mut outside = RunOutside {
            issue: self.issue,
            worktree: self.worktree,
            base: self.base,
            pr_url,
            base_fix: self.base_fix,
            sessions: self.sessions,
        };
        repair_loop::take_to_goal(&mut outside, self.base, goal)
    }

    fn delete_branch(&mut self) -> Result<()> {
        self.worktree.delete_from_origin()
    }

    fn issue_is_open(&mut self) -> Result<bool> {
        github::issue_is_open(self.issue)
    }

    fn close_issue(&mut self, comment: &str) -> Result<()> {
        github::close_issue(self.issue, comment)
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }

    fn warn(&mut self, error: &anyhow::Error, warning: String) {
        progress::warn(error, format_args!("{warning}"));
    }
}

/// A failed Delivery's worktree, which it owns, so it can keep it, and its
/// pull request on GitHub. The worktree is cleaned up, unless kept, when
/// this is dropped.
struct OnFailure<'a> {
    issue: &'a IssueUrl,
    base: &'a str,
    worktree: Worktree,
}

impl FailedOutside for OnFailure<'_> {
    /// Commit everything in the worktree, uncommitted work included, as the
    /// failure commit for `reason`, and push the Issue branch. An unfinished
    /// merge is aborted first. Does neither if the branch has no changes
    /// against the Base branch, so no empty Issue branch appears on origin.
    fn commit_and_push(&mut self, reason: &str) -> Result<()> {
        let worktree = &self.worktree;
        let git = worktree.git();
        if worktree.merge_in_progress()? {
            git.run(&["merge", "--abort"])?;
        }
        git.run(&["add", "-A"])?;
        let base = format!("origin/{}", self.base);
        if git.succeeds(&["diff", "--cached", "--quiet", &base])? {
            return Ok(());
        }
        let message = format!(
            "thirdshift: failed run ({reason})\n\n\
             {timestamp}, host {host}. Uncommitted work at the time of failure is included in this commit.",
            timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            host = host::name().as_deref().unwrap_or("unknown"),
        );
        // No hooks: a hook that rejects the commit would strand the work.
        git.run(&[
            "commit",
            "-q",
            "--allow-empty",
            "--no-verify",
            "-m",
            &message,
        ])?;
        worktree.push()
    }

    fn pull_request(&mut self) -> Result<Option<PullRequest>> {
        github::pull_request_for(self.issue, self.worktree.branch())
    }

    fn convert_to_draft(&mut self) -> Result<()> {
        github::convert_to_draft(self.issue, self.worktree.branch())
    }

    fn keep(&mut self) {
        self.worktree.keep();
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn step(&mut self, line: String) {
        progress::step(line);
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

/// Fail unless the PR for `branch` is still open, ready for review, and
/// mergeable, waiting up to the grace period for GitHub to work out the last.
fn ensure_pr_ready_and_mergeable(issue: &IssueUrl, branch: &str) -> Result<()> {
    progress::step("checking the PR is open, ready and mergeable");
    let pr = open(github::pull_request_for(issue, branch)?)?;
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
mod scripted {
    use std::path::PathBuf;

    use anyhow::{Result, anyhow, bail};

    use super::{FailedOutside, Outside};
    use crate::failed_run::PolicyRefusal;
    use crate::github::{PrState, PullRequest};
    use crate::run::Goal;

    /// A step of [`Outside`] or [`FailedOutside`], or the step before ready,
    /// that can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fails {
        CatchUp,
        Session,
        Push,
        BeforeReady,
        PullRequest,
        MarkReady,
        TakeToGoal,
        DeleteBranch,
        IssueIsOpen,
        CloseIssue,
        CommitAndPush,
        ConvertToDraft,
    }

    /// A call the Delivery made, in the order it made it.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Call {
        CatchUp,
        /// It ran the session of this kind.
        Session(String),
        Push,
        /// It ran the step before ready.
        BeforeReady,
        /// It read the pull request for the Issue branch.
        PullRequest,
        MarkReady,
        /// It took the pull request to this goal through the Repair loop.
        TakeToGoal(Goal),
        DeleteBranch,
        /// It read whether the issue is open.
        IssueIsOpen,
        /// It closed the issue with this comment.
        CloseIssue(String),
        /// It committed the work and pushed it, for this reason.
        CommitAndPush(String),
        ConvertToDraft,
        /// It kept the worktree.
        Keep,
        /// It asked whether an interrupt was requested.
        Interrupted,
        /// It handed on this progress line.
        Step(String),
        /// It handed on this error, then this warning.
        Warn {
            error: String,
            warning: String,
        },
    }

    /// The pull request for the Issue branch, as the script gives it.
    #[derive(Clone, Copy)]
    pub struct Pr {
        pub state: PrState,
        pub base: &'static str,
        pub draft: bool,
    }

    /// The outside world as a script answers it: the pull request, if there
    /// is one, whether the issue is open, whether an interrupt was
    /// requested, whether the Repair loop ends in a Policy refusal, and which
    /// steps fail, and how many times more.
    pub struct Scripted {
        pub pr: Option<Pr>,
        pub issue_open: bool,
        pub interrupted: bool,
        /// Whether taking the pull request to the goal fails with a Policy
        /// refusal.
        pub refused: bool,
        /// The error taking the pull request to the goal fails with, when it
        /// fails.
        pub repair_loop_error: &'static str,
        /// Each step that fails, with how many times more it does.
        pub failing: Vec<(Fails, usize)>,
        /// The log of the session started last, as the sessions' scope keeps
        /// it.
        pub log: Option<PathBuf>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
    }

    impl Default for Scripted {
        /// An open draft pull request into `main`, an open issue, and every
        /// step succeeding.
        fn default() -> Self {
            Scripted {
                pr: Some(Pr {
                    state: PrState::Open,
                    base: "main",
                    draft: true,
                }),
                issue_open: true,
                interrupted: false,
                refused: false,
                repair_loop_error: "CI failed",
                failing: Vec::new(),
                log: None,
                calls: Vec::new(),
            }
        }
    }

    impl Scripted {
        /// Make `what` fail every time.
        pub fn failing(self, what: Fails) -> Self {
            self.failing_times(what, usize::MAX)
        }

        /// Make `what` fail `times` times, then succeed.
        pub fn failing_times(mut self, what: Fails, times: usize) -> Self {
            self.failing.push((what, times));
            self
        }

        /// Fail if `what` is to fail, once fewer times from now on.
        pub fn fail_if(&mut self, what: Fails) -> Result<()> {
            match self
                .failing
                .iter_mut()
                .find(|(failing, _)| *failing == what)
            {
                Some((_, times)) if *times > 0 => {
                    *times -= 1;
                    bail!("{what:?} failed")
                }
                _ => Ok(()),
            }
        }

        fn read_pr(&self) -> Option<PullRequest> {
            self.pr.map(|pr| PullRequest {
                number: 1,
                url: "https://github.com/acme/widgets/pull/1".to_string(),
                state: pr.state,
                head: "issue-7".to_string(),
                base: pr.base.to_string(),
                is_draft: pr.draft,
            })
        }

        fn set_draft(&mut self, draft: bool) {
            if let Some(pr) = &mut self.pr {
                pr.draft = draft;
            }
        }
    }

    impl Outside for Scripted {
        fn catch_up(&mut self) -> Result<()> {
            self.calls.push(Call::CatchUp);
            self.fail_if(Fails::CatchUp)
        }

        fn session(&mut self, kind: &str, _prompt: &str) -> Result<()> {
            self.calls.push(Call::Session(kind.to_string()));
            self.log = Some(PathBuf::from(format!("/logs/7-{kind}.jsonl")));
            self.fail_if(Fails::Session)
        }

        fn push(&mut self) -> Result<()> {
            self.calls.push(Call::Push);
            self.fail_if(Fails::Push)
        }

        fn pull_request(&mut self) -> Result<Option<PullRequest>> {
            self.calls.push(Call::PullRequest);
            self.fail_if(Fails::PullRequest)?;
            Ok(self.read_pr())
        }

        fn mark_ready(&mut self) -> Result<()> {
            self.calls.push(Call::MarkReady);
            self.fail_if(Fails::MarkReady)?;
            self.set_draft(false);
            Ok(())
        }

        fn take_to_goal(&mut self, _pr_url: &str, goal: Goal) -> Result<()> {
            self.calls.push(Call::TakeToGoal(goal));
            if self.refused {
                return Err(anyhow!("Merge commits are not allowed").context(PolicyRefusal));
            }
            self.fail_if(Fails::TakeToGoal)
                .map_err(|_| anyhow!(self.repair_loop_error))
        }

        fn delete_branch(&mut self) -> Result<()> {
            self.calls.push(Call::DeleteBranch);
            self.fail_if(Fails::DeleteBranch)
        }

        fn issue_is_open(&mut self) -> Result<bool> {
            self.calls.push(Call::IssueIsOpen);
            self.fail_if(Fails::IssueIsOpen)?;
            Ok(self.issue_open)
        }

        fn close_issue(&mut self, comment: &str) -> Result<()> {
            self.calls.push(Call::CloseIssue(comment.to_string()));
            self.fail_if(Fails::CloseIssue)?;
            self.issue_open = false;
            Ok(())
        }

        fn interrupted(&mut self) -> bool {
            self.calls.push(Call::Interrupted);
            self.interrupted
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }

        fn warn(&mut self, error: &anyhow::Error, warning: String) {
            self.calls.push(Call::Warn {
                error: format!("{error:#}"),
                warning,
            });
        }
    }

    impl FailedOutside for Scripted {
        fn commit_and_push(&mut self, reason: &str) -> Result<()> {
            self.calls.push(Call::CommitAndPush(reason.to_string()));
            self.fail_if(Fails::CommitAndPush)
        }

        fn pull_request(&mut self) -> Result<Option<PullRequest>> {
            Outside::pull_request(self)
        }

        fn convert_to_draft(&mut self) -> Result<()> {
            self.calls.push(Call::ConvertToDraft);
            self.fail_if(Fails::ConvertToDraft)?;
            self.set_draft(true);
            Ok(())
        }

        fn keep(&mut self) {
            self.calls.push(Call::Keep);
        }

        fn interrupted(&mut self) -> bool {
            Outside::interrupted(self)
        }

        fn step(&mut self, line: String) {
            Outside::step(self, line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::scripted::{Call, Fails, Pr, Scripted};
    use super::*;
    use crate::github::PrState;

    fn seven() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
    }

    const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

    const CLOSING_COMMENT: &str = "Closed by #1, merged into main by a thirdshift Merge run.";

    /// Deliver the pull request for issue #7, from `issue-7` into `main`, to
    /// `goal` against `outside`, opening with the implement session, or with
    /// the Spec review after catching up from origin if `spec_pr`: its URL,
    /// or the Failed run, given the log of the session started last, as the
    /// sessions' scope gives it.
    fn deliver(goal: Goal, spec_pr: bool, outside: &mut Scripted) -> Result<String, FailedRun> {
        let issue = seven();
        let route = Route {
            issue: &issue,
            base: "main",
            branch: "issue-7",
            goal,
        };
        let opening = Opening {
            kind: if spec_pr { "spec-review" } else { "implement" },
            prompt: String::new(),
            catch_up_from_origin: spec_pr,
        };
        let delivered = route.steps(outside, &opening, |outside| {
            outside.calls.push(Call::BeforeReady);
            outside.fail_if(Fails::BeforeReady)
        });
        delivered.map_err(|error| {
            let log = outside.log.clone();
            fail(outside, log, error)
        })
    }

    /// The Failed run `outside` comes to, delivering to `goal`.
    fn failed(goal: Goal, outside: &mut Scripted) -> FailedRun {
        deliver(goal, false, outside).expect_err("the Delivery reached its goal")
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    fn session() -> Call {
        Call::Session("implement".to_string())
    }

    /// The calls up to the pull request taken to `goal`, from a draft.
    fn up_to_the_goal(goal: Goal) -> Vec<Call> {
        vec![
            session(),
            Call::Push,
            Call::BeforeReady,
            step("checking the PR"),
            Call::PullRequest,
            Call::MarkReady,
            Call::TakeToGoal(goal),
        ]
    }

    /// The calls after `up_to_the_goal`, from the merge on, for an open
    /// issue.
    fn after_the_merge() -> Vec<Call> {
        vec![
            Call::DeleteBranch,
            Call::IssueIsOpen,
            step("closing issue #7"),
            Call::CloseIssue(CLOSING_COMMENT.to_string()),
        ]
    }

    /// The calls after `call` in `outside`.
    fn after<'a>(outside: &'a Scripted, call: &Call) -> &'a [Call] {
        let at = outside
            .calls
            .iter()
            .rposition(|made| made == call)
            .unwrap_or_else(|| panic!("no {call:?} in {:?}", outside.calls));
        &outside.calls[at + 1..]
    }

    fn cause(failed: &FailedRun) -> String {
        format!("{:#}", failed.error)
    }

    #[test]
    fn a_ready_run_runs_its_steps_in_order_and_takes_none_after_a_merge() {
        let mut outside = Scripted::default();

        let delivered = deliver(Goal::ReadyForReview, false, &mut outside);

        assert_eq!(delivered.ok().as_deref(), Some(PR_URL));
        assert_eq!(outside.calls, up_to_the_goal(Goal::ReadyForReview));
    }

    #[test]
    fn a_merge_run_runs_the_same_steps_then_deletes_the_branch_and_closes_the_issue() {
        let mut outside = Scripted::default();

        let delivered = deliver(Goal::Merged, false, &mut outside);

        assert_eq!(delivered.ok().as_deref(), Some(PR_URL));
        let mut expected = up_to_the_goal(Goal::Merged);
        expected.extend(after_the_merge());
        assert_eq!(outside.calls, expected);
    }

    #[test]
    fn a_spec_pr_delivery_catches_up_from_origin_first() {
        let mut outside = Scripted::default();

        deliver(Goal::ReadyForReview, true, &mut outside)
            .ok()
            .unwrap();

        assert_eq!(
            outside.calls[..2],
            [Call::CatchUp, Call::Session("spec-review".to_string())]
        );
    }

    #[test]
    fn no_pr_fails() {
        let mut outside = Scripted {
            pr: None,
            ..Scripted::default()
        };

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(cause(&failed), "no PR found");
        assert!(
            !outside
                .calls
                .contains(&Call::TakeToGoal(Goal::ReadyForReview))
        );
    }

    #[test]
    fn a_pr_that_is_not_open_fails() {
        for (state, named) in [(PrState::Closed, "closed"), (PrState::Merged, "merged")] {
            let mut outside = Scripted {
                pr: Some(Pr {
                    state,
                    base: "main",
                    draft: false,
                }),
                ..Scripted::default()
            };

            let failed = failed(Goal::ReadyForReview, &mut outside);

            assert_eq!(cause(&failed), format!("PR {PR_URL} is {named}, not open"));
            assert!(!outside.calls.contains(&Call::MarkReady));
        }
    }

    #[test]
    fn a_pr_against_another_base_fails_naming_both_bases() {
        let mut outside = Scripted {
            pr: Some(Pr {
                state: PrState::Open,
                base: "develop",
                draft: true,
            }),
            ..Scripted::default()
        };

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(cause(&failed), "PR targets develop, not main");
        assert!(!outside.calls.contains(&Call::MarkReady));
    }

    #[test]
    fn a_draft_is_marked_ready() {
        let mut outside = Scripted::default();

        deliver(Goal::ReadyForReview, false, &mut outside)
            .ok()
            .unwrap();

        assert_eq!(
            after(&outside, &Call::PullRequest),
            [Call::MarkReady, Call::TakeToGoal(Goal::ReadyForReview)]
        );
    }

    #[test]
    fn a_pr_already_ready_gets_no_request() {
        let mut outside = Scripted {
            pr: Some(Pr {
                state: PrState::Open,
                base: "main",
                draft: false,
            }),
            ..Scripted::default()
        };

        deliver(Goal::ReadyForReview, false, &mut outside)
            .ok()
            .unwrap();

        assert_eq!(
            after(&outside, &Call::PullRequest),
            [Call::TakeToGoal(Goal::ReadyForReview)]
        );
    }

    #[test]
    fn an_issue_already_closed_is_not_closed_again() {
        let mut outside = Scripted {
            issue_open: false,
            ..Scripted::default()
        };

        deliver(Goal::Merged, false, &mut outside).ok().unwrap();

        assert_eq!(
            after(&outside, &Call::TakeToGoal(Goal::Merged)),
            [Call::DeleteBranch, Call::IssueIsOpen]
        );
    }

    #[test]
    fn a_failed_branch_deletion_is_a_warning_naming_the_push_to_run_by_hand() {
        let mut outside = Scripted::default().failing(Fails::DeleteBranch);

        let delivered = deliver(Goal::Merged, false, &mut outside);

        assert_eq!(delivered.ok().as_deref(), Some(PR_URL));
        assert_eq!(
            after(&outside, &Call::TakeToGoal(Goal::Merged))[..3],
            [
                Call::DeleteBranch,
                Call::Interrupted,
                Call::Warn {
                    error: "DeleteBranch failed".to_string(),
                    warning: "could not delete issue-7 on origin, so delete it by hand: \
                         git push origin --delete issue-7"
                        .to_string(),
                },
            ]
        );
        assert!(!outside.issue_open, "the issue is still closed");
    }

    #[test]
    fn a_failed_issue_close_is_a_warning_naming_the_close_with_its_comment_quoted() {
        for failing in [Fails::IssueIsOpen, Fails::CloseIssue] {
            let mut outside = Scripted::default().failing(failing);

            let delivered = deliver(Goal::Merged, false, &mut outside);

            assert_eq!(delivered.ok().as_deref(), Some(PR_URL), "{failing:?}");
            assert_eq!(
                outside.calls.last(),
                Some(&Call::Warn {
                    error: format!("{failing:?} failed"),
                    warning: format!(
                        "could not close issue #7, so if it is still open, close it by hand: \
                         gh issue close 7 --repo acme/widgets --comment '{CLOSING_COMMENT}'"
                    ),
                }),
                "{failing:?}"
            );
        }
    }

    #[test]
    fn a_step_after_the_merge_that_failed_while_interrupted_is_tried_once_more() {
        let mut outside = Scripted {
            interrupted: true,
            ..Scripted::default()
        }
        .failing_times(Fails::DeleteBranch, 1)
        .failing_times(Fails::CloseIssue, 1);

        let delivered = deliver(Goal::Merged, false, &mut outside);

        assert_eq!(delivered.ok().as_deref(), Some(PR_URL));
        let closing = || {
            [
                Call::IssueIsOpen,
                step("closing issue #7"),
                Call::CloseIssue(CLOSING_COMMENT.to_string()),
            ]
        };
        let mut expected = vec![Call::DeleteBranch, Call::Interrupted, Call::DeleteBranch];
        expected.extend(closing());
        expected.push(Call::Interrupted);
        expected.extend(closing());
        assert_eq!(after(&outside, &Call::TakeToGoal(Goal::Merged)), expected);
    }

    #[test]
    fn a_failure_at_each_step_goes_down_the_failed_run_path_with_the_right_session_log() {
        let session_log = Some(PathBuf::from("/logs/7-spec-review.jsonl"));
        for (failing, log) in [
            (Fails::CatchUp, None),
            (Fails::Session, session_log.clone()),
            (Fails::Push, session_log.clone()),
            (Fails::BeforeReady, session_log.clone()),
            (Fails::PullRequest, session_log.clone()),
            (Fails::MarkReady, session_log.clone()),
            (Fails::TakeToGoal, session_log.clone()),
        ] {
            let mut outside = Scripted::default().failing(failing);

            let failed = deliver(Goal::Merged, true, &mut outside).err().unwrap();

            let error = if failing == Fails::TakeToGoal {
                "CI failed".to_string()
            } else {
                format!("{failing:?} failed")
            };
            assert_eq!(cause(&failed), error, "{failing:?}");
            assert_eq!(failed.log, log, "{failing:?}");
            assert!(!failed.interrupted, "{failing:?}");
            assert!(
                outside.calls.contains(&Call::CommitAndPush(error)),
                "{failing:?}: {:?}",
                outside.calls
            );
            assert!(!outside.calls.contains(&Call::DeleteBranch), "{failing:?}");
        }
    }

    #[test]
    fn a_failure_goes_down_the_failed_run_path_in_order() {
        let mut outside = Scripted::default().failing(Fails::TakeToGoal);

        let failed = failed(Goal::Merged, &mut outside);

        assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
        assert_eq!(
            after(&outside, &Call::TakeToGoal(Goal::Merged)),
            [
                Call::Interrupted,
                Call::CommitAndPush("CI failed".to_string()),
                Call::PullRequest,
                Call::ConvertToDraft,
            ]
        );
    }

    #[test]
    fn an_interrupt_replaces_the_error() {
        let mut outside = Scripted {
            interrupted: true,
            ..Scripted::default()
        }
        .failing(Fails::Session);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(cause(&failed), "interrupted");
        assert!(failed.interrupted);
        assert!(
            outside
                .calls
                .contains(&Call::CommitAndPush("interrupted".to_string()))
        );
    }

    #[test]
    fn a_policy_refusal_neither_commits_pushes_nor_converts_and_keeps_the_pr_url() {
        let mut outside = Scripted {
            refused: true,
            ..Scripted::default()
        };

        let failed = failed(Goal::Merged, &mut outside);

        assert!(failed.error.is::<PolicyRefusal>());
        assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
        assert_eq!(
            after(&outside, &Call::TakeToGoal(Goal::Merged)),
            [Call::Interrupted, Call::PullRequest]
        );
    }

    #[test]
    fn the_failure_commit_gets_the_first_line_of_the_cause_as_its_reason() {
        let mut outside = Scripted {
            repair_loop_error: "test failed on main\nand on the Issue branch",
            ..Scripted::default()
        }
        .failing(Fails::TakeToGoal);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(
            cause(&failed),
            "test failed on main\nand on the Issue branch"
        );
        assert!(
            outside
                .calls
                .contains(&Call::CommitAndPush("test failed on main".to_string()))
        );
    }

    #[test]
    fn a_failed_push_keeps_the_worktree_and_says_so() {
        let mut outside = Scripted::default()
            .failing(Fails::Session)
            .failing(Fails::CommitAndPush);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(cause(&failed), "Session failed");
        assert_eq!(
            after(&outside, &Call::CommitAndPush("Session failed".to_string())),
            [
                step(
                    "could not push the failed run's work, so it may exist only locally: \
                     CommitAndPush failed"
                ),
                Call::PullRequest,
                Call::Keep,
            ]
        );
    }

    #[test]
    fn a_pushed_failure_does_not_keep_the_worktree() {
        let mut outside = Scripted::default().failing(Fails::Session);

        failed(Goal::ReadyForReview, &mut outside);

        assert!(!outside.calls.contains(&Call::Keep));
    }

    #[test]
    fn a_failed_draft_conversion_is_a_progress_line_and_the_run_fails_with_its_own_error() {
        let mut outside = Scripted::default()
            .failing(Fails::TakeToGoal)
            .failing(Fails::ConvertToDraft);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(cause(&failed), "CI failed");
        assert_eq!(
            after(&outside, &Call::ConvertToDraft),
            [step(
                "could not convert the PR to a draft: ConvertToDraft failed"
            )]
        );
    }

    #[test]
    fn an_open_draft_gets_no_conversion_request() {
        let mut outside = Scripted::default().failing(Fails::Session);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
        assert!(!outside.calls.contains(&Call::ConvertToDraft));
    }

    #[test]
    fn no_open_pr_gives_no_url() {
        for pr in [
            None,
            Some(Pr {
                state: PrState::Closed,
                base: "main",
                draft: false,
            }),
        ] {
            let mut outside = Scripted {
                pr,
                ..Scripted::default()
            }
            .failing(Fails::Session);

            let failed = failed(Goal::ReadyForReview, &mut outside);

            assert_eq!(failed.pr_url, None);
            assert!(!outside.calls.contains(&Call::ConvertToDraft));
        }
    }

    #[test]
    fn a_session_log_is_the_one_the_sessions_scope_gave() {
        let mut outside = Scripted::default().failing(Fails::Push);

        let failed = failed(Goal::ReadyForReview, &mut outside);

        assert_eq!(
            failed.log.as_deref(),
            Some(Path::new("/logs/7-implement.jsonl"))
        );
    }
}
