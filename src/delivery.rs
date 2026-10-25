//! Delivery: what a Run, or a Spec run for its Spec PR, does from its
//! worktree once its work begins. The opening session, the optional guidance
//! Security review, the push after it,
//! marking the pull request ready, the Repair loop that keeps it mergeable
//! and green, the Self-merge in a Merge run and its steps after the merge,
//! and the Failed run path when any of these fails.
//!
//! Its steps reach the worktree, the sessions, the Repair loop, GitHub, the
//! interrupt and the progress lines only through [`Outside`], and the Failed
//! run path, which runs once the sessions have ended, only through
//! [`FailedOutside`]: [`InWorktree`] and [`OnFailure`] do each for real;
//! `Scripted`, in tests, from a script, recording each call.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

mod repair_loop;
mod stage;

use crate::pull_request::{AlreadyMerged, Identified, MergeAttempt, PullRequest};

use crate::base_fix::{self, BaseFix};
use crate::child_run::ChildAsks;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{FailedRun, PolicyRefusal, interrupted_or};
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::Choice;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::prompt;
use crate::recovery::{self, Preservation, Recovery};
use crate::run::{Goal, Reached, Resolution};
use crate::run_ending::Cause;
use crate::security::review::{Hold as SecurityHold, Outcome as SecurityOutcome, Review};
use crate::session::{Kind, Logs, Sessions};
use crate::skills;
use crate::trust::{self, IssueText};
use crate::worktree::{ForeignCommits, Merge, PendingMerge, Preserved, Worktree};

use repair_loop::{Repair, Upstream};
use stage::{CommandLog, CommandRecord, Stage, Tracker};
/// A Delivery of the pull request for `issue`, the Run's issue or the Spec,
/// into the Base branch `base`, to `goal`.
pub struct Delivery<'a> {
    /// Whether the opening session is followed by a guidance Security review.
    pub security_review: bool,
    /// What the Run hands on to every child Run, a Base fix or a Ticket's
    /// Run, whose Harness, Model and Effort its own sessions run on too.
    pub child_asks: &'a ChildAsks,
    pub issue: &'a IssueUrl,
    pub base: &'a str,
    pub goal: Goal,
    /// The one Base fix the Delivery may start, or wait on, when its only red
    /// checks are Inherited failures.
    pub base_fix: &'a mut BaseFix,
    /// Where its Session traces go.
    pub logs: &'a Logs,
    /// When the Run began: an untrusted edit to `issue` made since then
    /// starts no further session.
    pub started: DateTime<Utc>,
}

/// The session a Delivery opens with, given its prompt.
pub struct Opening {
    pub session: OpeningSession,
    pub prompt: String,
}

/// Which session a Delivery opens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningSession {
    /// A Run's implement session.
    Implementation,
    /// A bug Ticket run diagnoses before implementing, and may need information.
    BugDiagnosis,
    /// A Spec run's Spec review, once its Tickets have landed.
    SpecReview,
}

impl OpeningSession {
    /// The session's kind.
    pub fn kind(self) -> Kind {
        match self {
            Self::Implementation | Self::BugDiagnosis => Kind::Implementation,
            Self::SpecReview => Kind::SpecReview,
        }
    }

    /// Whether the branch is first fast-forwarded to its head on origin, as
    /// the Spec branch is to the Tickets landed there.
    pub fn catches_up_from_origin(self) -> bool {
        self == Self::SpecReview
    }

    fn stage(self) -> Stage {
        match self {
            Self::Implementation | Self::BugDiagnosis => Stage::Implementation,
            Self::SpecReview => Stage::SpecReview,
        }
    }
}

impl Delivery<'_> {
    /// Take the pull request from the branch checked out in `worktree` to the
    /// goal. For a Spec review opening, first fast-forward the
    /// branch to origin. Then run the opening session and optional Security
    /// review, push the branch, for
    /// any commit the session left unpushed, write the line that says what it
    /// was built with in the pull request's body, only warning if that fails,
    /// restore the optional Tickets checklist, and mark the pull request ready,
    /// failing unless it exists, is open and targets the Base branch. Then keep it mergeable
    /// and its CI green through the Repair loop, and for [`Goal::Merged`],
    /// Self-merge it. A merge that fails goes back round the Repair loop and
    /// is tried again on the new head; if that round finds nothing to fix,
    /// this fails with a `PolicyRefusal`. Any failure goes through the Failed
    /// run path, which keeps the worktree if its work did not reach origin;
    /// otherwise it is cleaned up when this returns. A gate observing an
    /// already merged PR completes as merged only if that head includes all
    /// local work; no further publication or PR mutations are performed.
    pub fn deliver(
        self,
        worktree: Worktree,
        opening: Opening,
        pull_request: &mut PullRequest,
        checklist: Option<&str>,
    ) -> Result<Reached, FailedRun> {
        let tracker = Tracker::new(opening.session.stage(), CommandLog);
        let route = Route {
            security_review: self.security_review,
            issue: self.issue,
            base: self.base,
            branch: worktree.branch(),
            goal: self.goal,
            harness: &self.child_asks.harness,
        };
        pull_request.begin_delivery();
        let (issue, started, path) = (self.issue, self.started, worktree.path());
        let before_each = move || {
            no_untrusted_edit_since(issue, started)?;
            hand_over_issue(issue, path)
        };
        let (delivered, log) = Sessions::checked_within(
            self.logs,
            worktree.path(),
            &self.child_asks.harness,
            self.child_asks.security_review_harness.as_ref(),
            self.child_asks.security_review_default.as_ref(),
            &before_each,
            |sessions| {
                let mut outside = InWorktree {
                    tracker: &tracker,
                    issue: self.issue,
                    worktree: &worktree,
                    base: self.base,
                    base_fix: self.base_fix,
                    child_asks: self.child_asks,
                    sessions,
                    pull_request,
                };
                route.steps(&mut outside, &opening, checklist)
            },
        );
        let result = match delivered {
            Ok(resolution) => Ok(Reached {
                resolution,
                log,
                ticket_lines: Vec::new(),
            }),
            Err(error) => {
                if let Some(merged) = error.downcast_ref::<AlreadyMerged>()
                    && worktree.fully_landed_at(&merged.head).unwrap_or(false)
                {
                    progress::step("the pull request was merged while Delivery was running");
                    Ok(Reached {
                        resolution: Resolution::PullRequest {
                            url: merged.pr.url.clone(),
                            goal: Goal::Merged,
                        },
                        log,
                        ticket_lines: Vec::new(),
                    })
                } else {
                    let stopped_at = stopped_at(&tracker, &error);
                    let path = worktree.path().to_path_buf();
                    let mut outside = OnFailure {
                        base: self.base,
                        worktree,
                        pull_request,
                    };
                    Err(fail(&mut outside, stopped_at, path, log, error))
                }
            }
        };
        crate::logs::unaddressed_findings(pull_request.unaddressed_findings());
        result
    }
}

/// The steps of a Delivery of the pull request for `issue`, from `branch`
/// into the Base branch `base`, to `goal`, its sessions run on `harness`.
struct Route<'a> {
    security_review: bool,
    issue: &'a IssueUrl,
    base: &'a str,
    branch: &'a str,
    goal: Goal,
    harness: &'a Choice,
}

/// Read one of the two explicit diagnosis completion lines. Missing or
/// malformed handoffs fail instead of falling through to publication.
fn not_reproduced(issue: &IssueUrl, final_message: Option<&str>) -> Result<Option<String>> {
    let line = final_message
        .and_then(|message| message.trim_end().lines().last())
        .unwrap_or("");
    if line == prompt::BUG_REPRODUCED_LINE {
        return Ok(None);
    }
    let url = line
        .strip_prefix(prompt::BUG_NOT_REPRODUCED_LINE)
        .context("bug diagnosis ended without a reproduced or not reproduced completion line")?;
    anyhow::ensure!(
        url.split_once("#issuecomment-")
            .is_some_and(|(comment_issue, id)| {
                IssueUrl::parse(comment_issue)
                    .is_ok_and(|comment_issue| comment_issue.identity() == issue.identity())
                    && !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_digit())
                    && id.parse::<u64>().is_ok_and(|n| n > 0)
            }),
        "bug not reproduced ending must link a comment on {}",
        issue.url
    );
    Ok(Some(url.to_string()))
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
    ) -> Result<Resolution> {
        let session = opening.session;
        if session.catches_up_from_origin() {
            outside.catch_up()?;
        }
        outside.stage(session.stage());
        let final_message = outside.session(session.kind(), &opening.prompt)?;
        if session == OpeningSession::BugDiagnosis
            && let Some(comment_url) = not_reproduced(self.issue, final_message.as_deref())?
        {
            outside.confirm_needs_info()?;
            return Ok(Resolution::NotReproduced { comment_url });
        }
        let review = match self.security_review.then(|| {
            outside.stage(Stage::SecurityReview);
            outside.security_review(&prompt::security_review(self.issue, self.base, self.branch))
        }) {
            // Not a Security review outcome: the Run itself must stop.
            Some((Err(error), _)) if error.is::<UntrustedEdit>() => return Err(error),
            review => review.map(|(result, log)| SecurityOutcome::of(result, log)),
        };
        outside.stage(Stage::Push);
        outside.push()?;
        self.write_built_with(outside);
        outside.stage(Stage::PullRequestReadiness);
        let pr = outside.mark_pr_ready(checklist)?;
        let hold = review.as_ref().and_then(SecurityOutcome::hold);
        if let Some(review) = &review
            && let Err(error) = outside.record_security_review(review)
        {
            outside.warn(
                &error,
                "could not write the Security review outcome in the pull request's body"
                    .to_string(),
            );
        }
        // A Security hold prevents Self-merge, while ordinary readiness still
        // includes repairing conflicts and watching the branch's CI.
        let goal = if hold.is_some() {
            Goal::ReadyForReview
        } else {
            self.goal
        };
        outside.take_to_goal(&pr.url, goal)?;
        if let Some(hold) = hold {
            outside.step(hold.to_string());
            if self.goal == Goal::Merged {
                return Err(hold.into());
            }
        }
        if self.goal == Goal::Merged {
            self.after_merge(outside, &pr);
        }
        Ok(Resolution::PullRequest {
            url: pr.url,
            goal: self.goal,
        })
    }

    /// Write the line that says the pull request was built with the Harness
    /// in its body, in place of any thirdshift wrote there before, only
    /// warning if that fails, as the work is done without it. Nothing if
    /// there is no pull request: marking it ready says so.
    fn write_built_with(&self, outside: &mut impl Outside) {
        if let Err(error) = outside.write_built_with(self.harness) {
            outside.warn(
                &error,
                "could not write the Built-with line in the pull request's body".into(),
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
            "Closed by #{}, merged into {} by a thirdshift Ticket run with Self-merge.",
            pr.number, self.base
        );
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            self.close_unless_closed(outside, &comment)
        }) {
            let issue = self.issue;
            outside.warn(
                &error,
                format!(
                    "could not close issue #{number} ({url}), so if it is still open, close it by hand: \
                     gh issue close {number} --repo {repo} --comment '{quoted}'",
                    number = issue.number,
                    url = issue.url,
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
/// `PolicyRefusal` or `SecurityHold` neither pushes nor converts, so the PR stays ready on the
/// head whose CI was watched. Problems along the way are reported, not
/// raised, so the error is what the Run fails with. Worktree owns preservation
/// and retains work that may not have reached origin. `log` is the most recent
/// Session trace, if a session created one, which the Failed run reports unless
/// the error is a Security hold with its review's own Session trace: a hold is
/// raised only after the Repair loop, whose sessions may have run since.
/// `stopped_at` is the stage its recovery lines report; `worktree`, where work
/// preserved only locally is.
fn fail(
    outside: &mut impl FailedOutside,
    stopped_at: recovery::Stage,
    worktree: PathBuf,
    log: Option<PathBuf>,
    error: anyhow::Error,
) -> FailedRun {
    let interrupted = outside.interrupted();
    let error = interrupted_or(error, interrupted);
    // The cause as stderr gives it, down to what a session left running, cut
    // to its first line: the reason goes in the failure commit's subject.
    let reason = Cause::of(&error).first_line().to_string();
    // Everything is already on origin: the Repair loop pushed the head.
    let keep_ready = error.is::<PolicyRefusal>() || error.is::<SecurityHold>();
    let log = error
        .downcast_ref::<SecurityHold>()
        .and_then(SecurityHold::log)
        .map(Path::to_path_buf)
        .or(log);
    let preservation = if keep_ready {
        Preservation::Remote
    } else {
        match outside.preserve_failed_run(&reason) {
            Ok(Preserved::Pushed) => Preservation::Remote,
            Ok(Preserved::NoChanges) => Preservation::None,
            Err(problem) => {
                outside.step(format!(
                    "could not push the failed ticket run's work, so it may exist only locally: {problem:#}"
                ));
                Preservation::Local
            }
        }
    };
    let pr_url = match outside.finish_pr(keep_ready) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            outside.step(format!("could not convert the PR to a draft: {problem:#}"));
            None
        }
    };
    let mut recovery = Recovery::new(stopped_at, preservation);
    recovery.record_worktree(worktree);
    FailedRun {
        recovery: Some(Box::new(recovery)),
        error,
        pr_url,
        log,
        interrupted,
        ticket_lines: Vec::new(),
    }
}

/// The stage a Delivery that failed with `error` stopped at, from
/// `tracker`: its Security review if the error is a Security hold, found once
/// the pull request was ready.
fn stopped_at(tracker: &Tracker<impl CommandRecord>, error: &anyhow::Error) -> recovery::Stage {
    if error.is::<SecurityHold>() {
        tracker.stop_at_security_review();
    }
    tracker.stopped_at()
}

/// What a Delivery's steps do or read outside the Delivery, within its
/// sessions' scope.
trait Outside {
    /// Enter `stage`, before the step it names.
    fn stage(&mut self, stage: Stage);
    /// Fast-forward the Issue branch to its head on origin.
    fn catch_up(&mut self) -> Result<()>;
    /// Run a session of `kind` given `prompt`.
    fn session(&mut self, kind: Kind, prompt: &str) -> Result<Option<String>>;
    /// The diagnosis session completed its label handoff.
    fn confirm_needs_info(&mut self) -> Result<()>;
    /// Run the Security review given `prompt`, with its Session trace, its
    /// Resume's if it got one, if it started a session.
    fn security_review(&mut self, prompt: &str) -> (Result<Review>, Option<PathBuf>);
    fn record_security_review(&mut self, review: &SecurityOutcome) -> Result<()>;
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
    fn preserve_failed_run(&mut self, reason: &str) -> Result<Preserved>;
    /// Finish the captured PR through completion operations after preservation.
    fn finish_pr(&mut self, keep_ready: bool) -> Result<Option<String>>;
    /// Whether an interrupt was requested.
    fn interrupted(&mut self) -> bool;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// Fail with [`UntrustedEdit`] if `issue` has had an edit to its body or
/// title, since its Run `started`, by an account that is not a Trusted
/// account: a session started now would work from text no Trusted account
/// wrote.
fn no_untrusted_edit_since(issue: &IssueUrl, started: DateTime<Utc>) -> Result<()> {
    match trust::untrusted_edit(&GitHub::new(), issue, Some(started))? {
        Some(text) => Err(UntrustedEdit {
            number: issue.number,
            text,
        }
        .into()),
        None => Ok(()),
    }
}

/// Write `issue` to [`prompt::ISSUE_FILE`] in `worktree`, kept out of git,
/// for the session about to start to work from: its title, its body and only
/// the comments of Trusted accounts, as [`GitHub::issue_for_session`] gives
/// them. Written again before each session, so each has the latest.
fn hand_over_issue(issue: &IssueUrl, worktree: &Path) -> Result<()> {
    skills::exclude(&Git::new(worktree), &format!("/{}", prompt::ISSUE_FILE))?;
    let text = GitHub::new().issue_for_session(issue)?;
    let path = worktree.join(prompt::ISSUE_FILE);
    fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))
}

/// Issue `number`'s `text` was edited during its Run by an account that is
/// not a Trusted account, so the Run starts no further session.
#[derive(Debug)]
struct UntrustedEdit {
    number: u64,
    text: IssueText,
}

impl std::fmt::Display for UntrustedEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{} {} edited since the Ticket run started by an account that is not a Trusted account; starting no further session",
            self.number, self.text
        )
    }
}

impl std::error::Error for UntrustedEdit {}

/// A Delivery's worktree, its sessions, its Base fix, and its issue and pull
/// request on GitHub.
struct InWorktree<'a> {
    tracker: &'a Tracker,
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    base_fix: &'a mut BaseFix,
    /// What a Base fix it starts is handed.
    child_asks: &'a ChildAsks,
    sessions: &'a Sessions<'a>,
    pull_request: &'a mut PullRequest,
}

impl Outside for InWorktree<'_> {
    fn stage(&mut self, stage: Stage) {
        self.tracker.enter(stage);
    }

    fn catch_up(&mut self) -> Result<()> {
        self.worktree.fast_forward_to_origin()
    }

    fn session(&mut self, kind: Kind, prompt: &str) -> Result<Option<String>> {
        let final_message = self.sessions.run(kind, prompt)?;
        self.pull_request.ensure_actionable(false)?;
        Ok(final_message)
    }

    fn confirm_needs_info(&mut self) -> Result<()> {
        let labels = GitHub::new().issue_labels(self.issue)?;
        anyhow::ensure!(
            labels.has(crate::labels::NEEDS_INFO) && !labels.has(crate::labels::READY_FOR_AGENT),
            "bug not reproduced ending requires needs-info with ready-for-agent absent"
        );
        Ok(())
    }

    fn security_review(&mut self, prompt: &str) -> (Result<Review>, Option<PathBuf>) {
        let review = self.review_while_actionable(prompt);
        if !review
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<UntrustedEdit>())
        {
            self.pull_request
                .security_reviewed(review.as_ref().map_or(1, |review| review.findings.len()));
        }
        (review, self.sessions.last_log_of(Kind::SecurityReview))
    }

    fn record_security_review(&mut self, review: &SecurityOutcome) -> Result<()> {
        self.pull_request.record_security_review(&review.entry())
    }

    fn push(&mut self) -> Result<()> {
        self.pull_request.ensure_actionable(false)?;
        self.worktree
            .push_if_actionable(&mut || self.pull_request.ensure_actionable(false))
    }

    fn write_built_with(&mut self, choice: &Choice) -> Result<()> {
        self.pull_request.write_built_with(choice)
    }

    fn mark_pr_ready(&mut self, checklist: Option<&str>) -> Result<Identified> {
        self.pull_request.mark_ready(checklist)
    }

    fn take_to_goal(&mut self, pr_url: &str, goal: Goal) -> Result<()> {
        let mut outside = RunOutside {
            tracker: self.tracker,
            issue: self.issue,
            worktree: self.worktree,
            base: self.base,
            pr_url,
            base_fix: self.base_fix,
            child_asks: self.child_asks,
            sessions: self.sessions,
            pull_request: self.pull_request,
            ci_watch: ci::Watch::new(self.issue),
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

impl InWorktree<'_> {
    /// Run the Security review given `prompt`, failing unless the pull
    /// request is still actionable both before and after it.
    fn review_while_actionable(&mut self, prompt: &str) -> Result<Review> {
        self.pull_request.ensure_actionable(false)?;
        let review = crate::security::review::run(
            self.sessions,
            self.worktree,
            self.issue,
            self.base,
            prompt,
        )?;
        self.pull_request.ensure_actionable(false)?;
        Ok(review)
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
    fn preserve_failed_run(&mut self, reason: &str) -> Result<Preserved> {
        if let Err(error) = self.pull_request.ensure_preservable() {
            self.worktree.retain_local();
            return Err(error);
        }
        self.worktree
            .preserve_failed_run(self.base, reason, &mut || {
                self.pull_request.ensure_preservable()
            })
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
    tracker: &'a Tracker,
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    pr_url: &'a str,
    base_fix: &'a mut BaseFix,
    /// What a Base fix it starts is handed.
    child_asks: &'a ChildAsks,
    sessions: &'a Sessions<'a>,
    pull_request: &'a mut PullRequest,
    ci_watch: ci::Watch<'a>,
}

impl RunOutside<'_> {
    /// A Repair or Base fix may have reported a new Harness version. Refresh
    /// attribution while the PR is still actionable, before any Self-merge.
    fn refresh_built_with(&mut self) -> Result<()> {
        self.pull_request.ensure_actionable(true)?;
        if let Err(error) = self.pull_request.write_built_with(&self.child_asks.harness) {
            progress::warn(
                &error,
                format_args!("could not write the Built-with line in the pull request's body"),
            );
        }
        Ok(())
    }
}

impl repair_loop::Outside for RunOutside<'_> {
    type Pending = PendingMerge;

    fn stage(&mut self, stage: Stage) {
        self.tracker.enter(stage);
    }

    fn merge_base_branch(&mut self) -> Result<Merge> {
        self.pull_request.ensure_actionable(true)?;
        self.worktree.merge_base_branch(self.base)
    }

    fn merge_new_commits(&mut self, observed: ForeignCommits) -> Result<Merge> {
        self.worktree.merge_new_commits(observed)
    }

    fn ensure_merged(&mut self, pending: &PendingMerge) -> Result<()> {
        self.worktree.ensure_merged(pending)
    }

    fn push(&mut self) -> Result<()> {
        self.pull_request.ensure_actionable(true)?;
        self.worktree
            .push_if_actionable(&mut || self.pull_request.ensure_actionable(true))
    }

    fn head(&mut self) -> Result<String> {
        self.worktree.head()
    }

    fn same_tree_as(&mut self, commit: &str) -> Result<bool> {
        self.worktree.same_tree_as(commit)
    }

    fn base_branch_moved(&mut self) -> Result<bool> {
        self.worktree.base_branch_moved(self.base)
    }

    fn new_commits_on_origin(&mut self) -> Result<ForeignCommits> {
        self.pull_request.ensure_actionable(true)?;
        self.worktree.new_commits_on_origin()
    }

    fn assess_ci(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci> {
        self.ci_watch.assess(
            head,
            base_commit,
            &mut || self.pull_request.ensure_actionable(true),
            &mut |watched| Ok(self.worktree.new_commits_on_origin()?.origin_head() != watched),
        )
    }

    fn ci_fix_completed(&mut self, head: String, failed: FailedChecks) {
        self.ci_watch.after_repair(head, failed);
    }

    fn repair(&mut self, kind: Kind, repair: Repair) -> Result<()> {
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
        self.pull_request.ensure_actionable(true)?;
        let result = self.sessions.run(kind, &prompt);
        let refreshed = self.refresh_built_with();
        result.and(refreshed)
    }

    fn sees_inherited_failures(&mut self) -> bool {
        self.base_fix.sees_inherited_failures()
    }

    fn base_fix(&mut self, base_commit: &str, failed: &FailedChecks) -> Result<()> {
        let outside = base_fix::LaunchAndGitHub {
            launch: self.worktree.launch(),
            issue: self.issue,
            asks: self.child_asks,
        };
        let result = self
            .base_fix
            .fix(outside, self.pr_url, self.base, base_commit, failed);
        let refreshed = self.refresh_built_with();
        result.and(refreshed)
    }

    fn pr_mergeable(&mut self) -> Result<bool> {
        self.pull_request.is_mergeable()
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
    use stage::Recorded;

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
        Session(Kind),
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
        /// The stages entered, apart from the calls.
        stages: Vec<Stage>,
        /// Whether the Delivery runs a Security review, and whether that
        /// review ends incomplete, a Security hold.
        reviews_security: bool,
        security_held: bool,
        /// Whether taking the PR to its goal runs a Repair session first.
        repairs: bool,
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
        fn record_security_review(&mut self, _: &SecurityOutcome) -> Result<()> {
            Ok(())
        }
        fn stage(&mut self, stage: Stage) {
            self.stages.push(stage);
        }
        fn security_review(&mut self, _: &str) -> (Result<Review>, Option<PathBuf>) {
            let log = PathBuf::from("/logs/security-review.jsonl");
            self.log = Some(log.clone());
            let review = if self.security_held {
                Review::from_final_message(Some("Security review: incomplete: timed out"))
            } else {
                Review::from_final_message(Some(
                    "Security review: {\"unaddressed_count\":0,\"findings\":[],\"pre_existing_count\":0}",
                ))
            };
            (review, Some(log))
        }
        fn catch_up(&mut self) -> Result<()> {
            self.calls.push(Call::CatchUp);
            self.check(Fails::CatchUp)
        }
        fn session(&mut self, kind: Kind, _: &str) -> Result<Option<String>> {
            self.calls.push(Call::Session(kind));
            self.log = Some(PathBuf::from(format!("/logs/{}.jsonl", kind.token())));
            self.check(Fails::Session).map(|()| None)
        }
        fn confirm_needs_info(&mut self) -> Result<()> {
            Ok(())
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
            if self.repairs {
                self.log = Some(PathBuf::from("/logs/repair-1.jsonl"));
            }
            if self.refused {
                return Err(anyhow!("merge refused").context(PolicyRefusal::NothingToFix));
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
        fn preserve_failed_run(&mut self, reason: &str) -> Result<Preserved> {
            self.calls.push(Call::Preserve(reason.to_string()));
            self.check(Fails::Preserve)?;
            Ok(Preserved::Pushed)
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
            security_review: outside.reviews_security,
            issue: &issue,
            base: "main",
            branch: "issue-7",
            goal,
            harness: &Choice::default(),
        };
        let opening = Opening {
            session: if spec {
                OpeningSession::SpecReview
            } else {
                OpeningSession::Implementation
            },
            prompt: String::new(),
        };
        route
            .steps(outside, &opening, spec.then_some("Tickets"))
            .map(|resolution| resolution.url().to_string())
            .map_err(|error| {
                // The real adapters enter each stage the steps name.
                let tracker = Tracker::new(opening.session.stage(), Recorded::default());
                for stage in &outside.stages {
                    tracker.enter(*stage);
                }
                let stopped_at = stopped_at(&tracker, &error);
                fail(
                    outside,
                    stopped_at,
                    PathBuf::from("/worktree"),
                    outside.log.clone(),
                    error,
                )
            })
    }

    /// The stage `failed` stopped at.
    fn stopped(failed: &FailedRun) -> recovery::Stage {
        failed.recovery.as_ref().expect("no recovery").stage()
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
                Call::Session(Kind::SpecReview),
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
            [Call::CatchUp, Call::Session(Kind::SpecReview)]
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
        assert!(
            outside.calls.contains(&Call::Close(
                "Closed by #12, merged into main by a thirdshift Ticket run with Self-merge."
                    .to_string()
            ))
        );
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
                recovery::Stage::Implementation,
                PathBuf::from("/worktree"),
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

    #[test]
    fn a_delivery_enters_each_stage_before_its_step() {
        let mut outside = Scripted {
            reviews_security: true,
            ..Default::default()
        };
        assert!(deliver(&mut outside, Goal::ReadyForReview, false).is_ok());
        assert_eq!(
            outside.stages,
            [
                Stage::Implementation,
                Stage::SecurityReview,
                Stage::Push,
                Stage::PullRequestReadiness
            ]
        );
    }

    #[test]
    fn a_spec_delivery_opens_in_its_spec_review() {
        for failing in [Fails::CatchUp, Fails::Session] {
            let mut outside = Scripted {
                failing: Some(failing),
                ..Default::default()
            };
            let failed = deliver(&mut outside, Goal::ReadyForReview, true).unwrap_err();
            assert_eq!(stopped(&failed), recovery::Stage::SpecReview, "{failing:?}");
        }
    }

    #[test]
    fn each_failed_step_stops_at_its_stage() {
        for (failing, stage) in [
            (Fails::Session, recovery::Stage::Implementation),
            (Fails::Push, recovery::Stage::Push),
            (Fails::Ready, recovery::Stage::PullRequest),
        ] {
            let mut outside = Scripted {
                failing: Some(failing),
                ..Default::default()
            };
            let failed = deliver(&mut outside, Goal::ReadyForReview, false).unwrap_err();
            assert_eq!(stopped(&failed), stage, "{failing:?}");
        }
    }

    #[test]
    fn a_security_hold_stops_at_the_security_review_without_entering_it_again() {
        let mut outside = Scripted {
            reviews_security: true,
            security_held: true,
            ..Default::default()
        };
        let failed = deliver(&mut outside, Goal::Merged, false).unwrap_err();
        assert!(failed.error.is::<SecurityHold>(), "{:#}", failed.error);
        assert_eq!(stopped(&failed), recovery::Stage::SecurityReview);
        assert_eq!(
            outside.stages,
            [
                Stage::Implementation,
                Stage::SecurityReview,
                Stage::Push,
                Stage::PullRequestReadiness
            ]
        );
    }

    #[test]
    fn a_security_hold_after_a_repair_reports_the_reviews_session_log() {
        let mut outside = Scripted {
            reviews_security: true,
            security_held: true,
            repairs: true,
            ..Default::default()
        };
        let failed = deliver(&mut outside, Goal::Merged, false).unwrap_err();
        assert!(failed.error.is::<SecurityHold>(), "{:#}", failed.error);
        assert_eq!(outside.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
        assert_eq!(
            failed.log,
            Some(PathBuf::from("/logs/security-review.jsonl"))
        );
    }

    #[test]
    fn a_failure_after_a_repair_reports_the_most_recent_session_log() {
        // A failing Repair session, or a cause no session gave, as with
        // Repairs exhausted, is reported with the most recent Session trace,
        // whether or not a Security review ran before it.
        for reviews_security in [false, true] {
            let mut outside = Scripted {
                reviews_security,
                repairs: true,
                failing: Some(Fails::Goal),
                ..Default::default()
            };
            let failed = deliver(&mut outside, Goal::Merged, false).unwrap_err();
            assert_eq!(failed.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
        }
    }

    #[test]
    fn a_security_hold_whose_review_started_no_session_reports_the_most_recent_session_log() {
        let hold = SecurityOutcome::of(Err(anyhow!("no merge base")), None)
            .hold()
            .unwrap();
        let failed = fail(
            &mut Scripted::default(),
            recovery::Stage::SecurityReview,
            PathBuf::from("/worktree"),
            Some(PathBuf::from("/logs/repair-1.jsonl")),
            hold.into(),
        );
        assert_eq!(failed.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
    }
}
