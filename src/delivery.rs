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

use anyhow::Result;

mod repair_loop;

use crate::pull_request::{Identified, MergeAttempt, PullRequest};

use crate::base_fix::BaseFix;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{FailedRun, PolicyRefusal, interrupted_or};
use crate::github::GitHub;
use crate::harness::Choice;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::prompt;
use crate::run::{Goal, Reached};
use crate::run_ending::Cause;
use crate::session::{Logs, Sessions};
use crate::worktree::{ForeignCommits, Merge, PendingMerge, Worktree};

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
    /// The Harness, Model and Effort its sessions run on, and its child
    /// Runs' too.
    pub harness: &'a Choice,
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
    /// any commit the session left unpushed, write the line that says what it
    /// was built with in the pull request's body, only warning if that fails,
    /// restore the optional Tickets checklist, and mark the pull request ready,
    /// failing unless it exists, is open and targets the Base branch. Then keep it mergeable
    /// and its CI green through the Repair loop, and for [`Goal::Merged`],
    /// Self-merge it. A merge that fails goes back round the Repair loop and
    /// is tried again on the new head; if that round finds nothing to fix,
    /// this fails with a `PolicyRefusal`. Any failure goes through the Failed
    /// run path, which keeps the worktree if its work did not reach origin;
    /// otherwise it is cleaned up when this returns.
    pub fn deliver(
        self,
        worktree: Worktree,
        opening: Opening,
        pull_request: &mut PullRequest,
        checklist: Option<&str>,
    ) -> Result<Reached, FailedRun> {
        let route = Route {
            issue: self.issue,
            base: self.base,
            branch: worktree.branch(),
            goal: self.goal,
            harness: self.harness,
        };
        pull_request.begin_delivery();
        let (delivered, log) =
            Sessions::within(self.logs, worktree.path(), self.harness, |sessions| {
                let mut outside = InWorktree {
                    issue: self.issue,
                    worktree: &worktree,
                    base: self.base,
                    base_fix: self.base_fix,
                    sessions,
                    pull_request,
                };
                route.steps(&mut outside, &opening, checklist)
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
                    base: self.base,
                    worktree,
                    pull_request,
                };
                Err(fail(&mut outside, log, error))
            }
        }
    }
}

/// The steps of a Delivery of the pull request for `issue`, from `branch`
/// into the Base branch `base`, to `goal`, its sessions run on `harness`.
struct Route<'a> {
    issue: &'a IssueUrl,
    base: &'a str,
    branch: &'a str,
    goal: Goal,
    harness: &'a Choice,
}

impl Route<'_> {
    /// [`Delivery::deliver`]'s steps, through `outside`, in order: catch up
    /// from origin if `opening` says to, the opening session, the push, the
    /// line saying what it was built with, checklist restoration, marking the
    /// pull request ready, taking it to the goal through the Repair loop, then for [`Goal::Merged`], the steps after
    /// the merge. Returns the pull request's URL.
    fn steps<O: Outside>(
        &self,
        outside: &mut O,
        opening: &Opening,
        checklist: Option<&str>,
    ) -> Result<String> {
        if opening.catch_up_from_origin {
            outside.catch_up()?;
        }
        outside.session(opening.kind, &opening.prompt)?;
        outside.push()?;
        self.write_built_with(outside);
        let pr = outside.mark_pr_ready(checklist)?;
        outside.take_to_goal(&pr.url, self.goal)?;
        if self.goal == Goal::Merged {
            self.after_merge(outside, &pr);
        }
        Ok(pr.url)
    }

    /// Write the line that says the pull request was built with the Harness
    /// in its body, in place of any thirdshift wrote there before, only
    /// warning if that fails, as the work is done without it. Nothing if
    /// there is no pull request: marking it ready says so.
    fn write_built_with(&self, outside: &mut impl Outside) {
        let built_with = self.harness.built_with();
        if let Err(error) = outside.write_built_with(self.harness) {
            outside.warn(
                &error,
                format!("could not write \"{built_with}\" in the pull request's body"),
            );
        }
    }

    /// The Self-merge's steps after the merge of `pr`: delete the Issue
    /// branch on origin, and close the issue unless it is closed already.
    /// GitHub may close it too, a moment after the merge, or may not, so
    /// thirdshift does not wait to see. The merge can't be undone, so these
    /// never fail the Run, and an interrupt no longer stops it: a step that
    /// fails is tried once more if an interrupt was requested, then is a
    /// warning naming the fix to make by hand.
    fn after_merge(&self, outside: &mut impl Outside, pr: &Identified) {
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
            pr.number, self.base
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

/// Take the Run down the Failed run path through `outside`, once its
/// sessions have ended with `error`: commit and push the work, and send an
/// open PR back to draft. An interrupt, if one was requested, is the error
/// instead: it can surface as some other error, such as a killed git. A
/// `PolicyRefusal` neither pushes nor converts, so the PR stays ready on the
/// head whose CI was watched. Problems along the way are reported, not
/// raised, so the error is what the Run fails with. Worktree owns preservation
/// and retains work that may not have reached origin. `log` is the most recent
/// Session log, if a session created one.
fn fail(outside: &mut impl FailedOutside, log: Option<PathBuf>, error: anyhow::Error) -> FailedRun {
    let interrupted = outside.interrupted();
    let error = interrupted_or(error, interrupted);
    // The cause as stderr gives it, down to what a session left running, cut
    // to its first line: the reason goes in the failure commit's subject.
    let reason = Cause::of(&error).first_line().to_string();
    // Everything is already on origin: the Repair loop pushed the head.
    let keep_ready = error.is::<PolicyRefusal>();
    if !keep_ready && let Err(problem) = outside.preserve_failed_run(&reason) {
        outside.step(format!(
            "could not push the failed run's work, so it may exist only locally: {problem:#}"
        ));
    }
    let pr_url = match outside.finish_pr(keep_ready) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            outside.step(format!("could not convert the PR to a draft: {problem:#}"));
            None
        }
    };
    FailedRun {
        error,
        pr_url,
        log,
        interrupted,
        ticket_lines: Vec::new(),
    }
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
    /// Write the annotation through the captured pull request module.
    fn write_built_with(&mut self, choice: &Choice) -> Result<()>;
    /// Validate and mark the captured pull request ready.
    fn mark_pr_ready(&mut self, checklist: Option<&str>) -> Result<Identified>;
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
    /// Preserve the Failed run's work for `reason`, retaining it locally if
    /// preservation fails.
    fn preserve_failed_run(&mut self, reason: &str) -> Result<()>;
    /// Finish the captured PR through completion operations after preservation.
    fn finish_pr(&mut self, keep_ready: bool) -> Result<Option<String>>;
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
    pull_request: &'a mut PullRequest,
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

    fn write_built_with(&mut self, choice: &Choice) -> Result<()> {
        self.pull_request.write_built_with(choice)
    }

    fn mark_pr_ready(&mut self, checklist: Option<&str>) -> Result<Identified> {
        self.pull_request.mark_ready(checklist)
    }

    fn take_to_goal(&mut self, pr_url: &str, goal: Goal) -> Result<()> {
        let mut outside = RunOutside {
            issue: self.issue,
            worktree: self.worktree,
            base: self.base,
            pr_url,
            base_fix: self.base_fix,
            sessions: self.sessions,
            pull_request: self.pull_request,
        };
        repair_loop::take_to_goal(&mut outside, self.base, goal)
    }

    fn delete_branch(&mut self) -> Result<()> {
        self.worktree.delete_from_origin()
    }

    fn issue_is_open(&mut self) -> Result<bool> {
        GitHub::new().completion().issue_is_open(self.issue)
    }

    fn close_issue(&mut self, comment: &str) -> Result<()> {
        GitHub::new().completion().close_issue(self.issue, comment)
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

/// A failed Delivery's worktree and its pull request on GitHub. Worktree
/// owns preservation and the cleanup decision when this is dropped.
struct OnFailure<'a> {
    base: &'a str,
    worktree: Worktree,
    pull_request: &'a mut PullRequest,
}

impl FailedOutside for OnFailure<'_> {
    fn preserve_failed_run(&mut self, reason: &str) -> Result<()> {
        self.worktree.preserve_failed_run(self.base, reason)
    }

    fn finish_pr(&mut self, keep_ready: bool) -> Result<Option<String>> {
        self.pull_request.finish_failed_run(keep_ready)
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
    pull_request: &'a mut PullRequest,
}

impl repair_loop::Outside for RunOutside<'_> {
    type Pending = PendingMerge;

    fn merge_base_branch(&mut self) -> Result<Merge> {
        self.worktree.merge_base_branch(self.base)
    }

    fn merge_new_commits(&mut self, observed: ForeignCommits) -> Result<Merge> {
        self.worktree.merge_new_commits(observed)
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

    fn base_branch_moved(&mut self) -> Result<bool> {
        self.worktree.base_branch_moved(self.base)
    }

    fn new_commits_on_origin(&mut self) -> Result<ForeignCommits> {
        self.worktree.new_commits_on_origin()
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
        self.pull_request.ensure_ready_and_mergeable()
    }

    fn merge(&mut self, head: &str) -> Result<MergeAttempt> {
        self.pull_request.merge(head)
    }

    fn interrupt_requested(&mut self) -> bool {
        interrupt::requested()
    }

    fn progress(&mut self, line: String) {
        progress::step(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{anyhow, bail};

    const PR_URL: &str = "https://github.com/acme/widgets/pull/12";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Fails {
        CatchUp,
        Session,
        Push,
        Annotation,
        Checklist,
        Ready,
        Goal,
        Preserve,
        Finish,
        Delete,
        Close,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        CatchUp,
        Session(String),
        Push,
        Annotation,
        Checklist,
        Ready,
        Goal(Goal),
        Preserve(String),
        Finish(bool),
        Delete,
        Close(String),
        Warning(String),
        Step(String),
    }

    /// Orchestration adapter: PR policy is tested through PullRequest's
    /// interface, so this only scripts the outcomes Delivery consumes.
    #[derive(Default)]
    struct Scripted {
        calls: Vec<Call>,
        failing: Option<Fails>,
        interrupted: bool,
        refused: bool,
        issue_closed: bool,
        log: Option<PathBuf>,
    }

    impl Scripted {
        fn check(&self, at: Fails) -> Result<()> {
            if self.failing == Some(at) {
                bail!("{at:?} failed");
            }
            Ok(())
        }
    }

    impl Outside for Scripted {
        fn catch_up(&mut self) -> Result<()> {
            self.calls.push(Call::CatchUp);
            self.check(Fails::CatchUp)
        }
        fn session(&mut self, kind: &str, _: &str) -> Result<()> {
            self.calls.push(Call::Session(kind.to_string()));
            self.log = Some(PathBuf::from(format!("/logs/{kind}.jsonl")));
            self.check(Fails::Session)
        }
        fn push(&mut self) -> Result<()> {
            self.calls.push(Call::Push);
            self.check(Fails::Push)
        }
        fn write_built_with(&mut self, _: &Choice) -> Result<()> {
            self.calls.push(Call::Annotation);
            self.check(Fails::Annotation)
        }
        fn mark_pr_ready(&mut self, checklist: Option<&str>) -> Result<Identified> {
            if checklist.is_some() {
                self.calls.push(Call::Checklist);
                self.check(Fails::Checklist)?;
            }
            self.calls.push(Call::Ready);
            self.check(Fails::Ready)?;
            Ok(Identified {
                number: 12,
                url: PR_URL.to_string(),
            })
        }
        fn take_to_goal(&mut self, _: &str, goal: Goal) -> Result<()> {
            self.calls.push(Call::Goal(goal));
            if self.refused {
                return Err(anyhow!("merge refused").context(PolicyRefusal));
            }
            self.check(Fails::Goal)
        }
        fn delete_branch(&mut self) -> Result<()> {
            self.calls.push(Call::Delete);
            self.check(Fails::Delete)
        }
        fn issue_is_open(&mut self) -> Result<bool> {
            Ok(!self.issue_closed)
        }
        fn close_issue(&mut self, comment: &str) -> Result<()> {
            self.calls.push(Call::Close(comment.to_string()));
            self.check(Fails::Close)
        }
        fn interrupted(&mut self) -> bool {
            self.interrupted
        }
        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
        fn warn(&mut self, _: &anyhow::Error, warning: String) {
            self.calls.push(Call::Warning(warning));
        }
    }

    impl FailedOutside for Scripted {
        fn preserve_failed_run(&mut self, reason: &str) -> Result<()> {
            self.calls.push(Call::Preserve(reason.to_string()));
            self.check(Fails::Preserve)
        }
        fn finish_pr(&mut self, keep_ready: bool) -> Result<Option<String>> {
            self.calls.push(Call::Finish(keep_ready));
            self.check(Fails::Finish)?;
            Ok(Some(PR_URL.to_string()))
        }
        fn interrupted(&mut self) -> bool {
            self.interrupted
        }
        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
    }

    fn deliver(outside: &mut Scripted, goal: Goal, spec: bool) -> Result<String, FailedRun> {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let route = Route {
            issue: &issue,
            base: "main",
            branch: "issue-7",
            goal,
            harness: &Choice::default(),
        };
        let opening = Opening {
            kind: if spec { "spec-review" } else { "implement" },
            prompt: String::new(),
            catch_up_from_origin: spec,
        };
        route
            .steps(outside, &opening, spec.then_some("Tickets"))
            .map_err(|error| fail(outside, outside.log.clone(), error))
    }

    #[test]
    fn spec_delivery_pushes_then_annotates_then_restores_checklist_before_readiness() {
        let mut outside = Scripted::default();
        assert_eq!(
            deliver(&mut outside, Goal::ReadyForReview, true)
                .ok()
                .as_deref(),
            Some(PR_URL)
        );
        assert_eq!(
            outside.calls,
            [
                Call::CatchUp,
                Call::Session("spec-review".to_string()),
                Call::Push,
                Call::Annotation,
                Call::Checklist,
                Call::Ready,
                Call::Goal(Goal::ReadyForReview)
            ]
        );
    }

    #[test]
    fn spec_delivery_catches_up_before_its_opening_session() {
        let mut outside = Scripted::default();
        assert!(deliver(&mut outside, Goal::ReadyForReview, true).is_ok());
        assert_eq!(
            outside.calls[..2],
            [Call::CatchUp, Call::Session("spec-review".to_string())]
        );
    }

    #[test]
    fn annotation_failure_warns_and_still_restores_checklist_and_marks_ready() {
        let mut outside = Scripted {
            failing: Some(Fails::Annotation),
            ..Default::default()
        };
        assert!(deliver(&mut outside, Goal::ReadyForReview, true).is_ok());
        assert!(
            matches!(&outside.calls[4], Call::Warning(line) if line.contains("could not write"))
        );
        assert_eq!(
            outside.calls[5..],
            [
                Call::Checklist,
                Call::Ready,
                Call::Goal(Goal::ReadyForReview)
            ]
        );
    }

    #[test]
    fn only_confirmed_self_merge_deletes_the_branch_and_closes_the_issue() {
        let mut outside = Scripted::default();
        assert!(deliver(&mut outside, Goal::Merged, false).is_ok());
        assert!(outside.calls.contains(&Call::Delete));
        assert!(outside.calls.contains(&Call::Close(
            "Closed by #12, merged into main by a thirdshift Merge run.".to_string()
        )));
    }

    #[test]
    fn each_delivery_failure_preserves_work_then_finishes_the_pr_with_its_primary_cause() {
        for failing in [
            Fails::CatchUp,
            Fails::Session,
            Fails::Push,
            Fails::Checklist,
            Fails::Ready,
            Fails::Goal,
        ] {
            let mut outside = Scripted {
                failing: Some(failing),
                ..Default::default()
            };
            let failed = deliver(&mut outside, Goal::Merged, true).unwrap_err();
            assert_eq!(failed.error.to_string(), format!("{failing:?} failed"));
            assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
            assert_eq!(
                failed.log,
                if failing == Fails::CatchUp {
                    None
                } else {
                    Some(PathBuf::from("/logs/spec-review.jsonl"))
                }
            );
            assert_eq!(
                outside.calls[outside.calls.len() - 2..],
                [
                    Call::Preserve(format!("{failing:?} failed")),
                    Call::Finish(false)
                ]
            );
            assert!(!outside.calls.contains(&Call::Delete));
        }
    }

    #[test]
    fn interruption_preserves_work_and_keeps_the_recorded_interruption() {
        let mut outside = Scripted {
            failing: Some(Fails::Session),
            interrupted: true,
            ..Default::default()
        };
        let failed = deliver(&mut outside, Goal::ReadyForReview, false).unwrap_err();
        assert_eq!(failed.error.to_string(), "interrupted");
        assert!(failed.interrupted);
        assert!(
            outside
                .calls
                .contains(&Call::Preserve("interrupted".to_string()))
        );
        assert!(outside.calls.contains(&Call::Finish(false)));
    }

    #[test]
    fn policy_refusal_keeps_the_pr_ready_without_a_preservation_push() {
        let mut outside = Scripted {
            refused: true,
            ..Default::default()
        };
        let failed = deliver(&mut outside, Goal::Merged, false).unwrap_err();
        assert!(failed.error.is::<PolicyRefusal>());
        assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
        assert!(outside.calls.contains(&Call::Finish(true)));
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::Preserve(_)))
        );
    }

    #[test]
    fn failed_run_finishing_errors_warn_and_preserve_the_primary_error_and_log() {
        for failing in [Fails::Preserve, Fails::Finish] {
            let mut outside = Scripted {
                failing: Some(failing),
                ..Default::default()
            };
            let failed = fail(
                &mut outside,
                Some(PathBuf::from("/logs/implement.jsonl")),
                anyhow!("original\nextra context"),
            );
            assert_eq!(failed.error.to_string(), "original\nextra context");
            assert_eq!(failed.log, Some(PathBuf::from("/logs/implement.jsonl")));
            assert_eq!(outside.calls[0], Call::Preserve("original".to_string()));
            assert!(
                outside
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Step(line) if line.contains("could not")))
            );
        }
    }

    #[test]
    fn post_merge_errors_are_warnings_and_an_already_closed_issue_stays_closed() {
        for failing in [Fails::Delete, Fails::Close] {
            let mut outside = Scripted {
                failing: Some(failing),
                ..Default::default()
            };
            assert!(deliver(&mut outside, Goal::Merged, false).is_ok());
            assert!(
                outside
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Warning(_)))
            );
        }
        let mut outside = Scripted {
            issue_closed: true,
            ..Default::default()
        };
        assert!(deliver(&mut outside, Goal::Merged, false).is_ok());
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::Close(_)))
        );
    }
}
