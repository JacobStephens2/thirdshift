//! Delivery: publication and recovery shared by Process Steps and the Spec PR.
//! The Ticket run's Process file owns its sequence. A Spec PR's Delivery
//! opens with the Spec review and optional Security review, then shares the push,
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

pub(crate) use repair_loop::MAX_REPAIRS;

/// Tightening policies inherited by dispatched Runs and their child Tickets.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub repair_cap: usize,
    pub protected_paths: Vec<String>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            repair_cap: MAX_REPAIRS,
            protected_paths: Vec::new(),
        }
    }
}

use crate::pull_request::{AlreadyMerged, Identified, MergeAttempt, PullRequest};

use crate::base_fix::{self, BaseFix};
use crate::child_run::ChildAsks;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{FailedRun, interrupted_or};
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
use crate::security::review::{Hold as SecurityHold, Outcome as SecurityOutcome, step};
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
    /// The most Repair sessions this Delivery starts. A Ticket run uses
    /// [`MAX_REPAIRS`]. A Custom process may set a lower cap.
    pub repair_cap: usize,
    /// Globs checked against the change before a Self-merge. A match holds
    /// the merge. Empty for a Ticket run.
    pub protected_paths: &'a [String],
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

    fn stage(self) -> Stage {
        match self {
            Self::Implementation | Self::BugDiagnosis => Stage::Implementation,
            Self::SpecReview => Stage::SpecReview,
        }
    }
}

impl Delivery<'_> {
    /// Take the Spec PR from the branch checked out in `worktree` to the
    /// goal. First fast-forward the branch to origin.
    /// Then run the Spec review and optional Security
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
    pub fn spec_review(
        self,
        worktree: Worktree,
        prompt: String,
        pull_request: &mut PullRequest,
        checklist: &str,
    ) -> Result<Reached, FailedRun> {
        self.deliver_steps(
            worktree,
            Opening {
                session: OpeningSession::SpecReview,
                prompt,
            },
            pull_request,
            Some(checklist),
            None,
        )
    }

    /// Execute the compiled Ticket process inside the same checked session
    /// and Failed ticket run lifecycle as the original Ticket run.
    pub fn ticket_run(
        self,
        worktree: Worktree,
        opening: Opening,
        pull_request: &mut PullRequest,
    ) -> Result<Reached, FailedRun> {
        let steps = crate::processes::ticket::steps()?;
        self.deliver_steps(worktree, opening, pull_request, None, Some(steps))
    }

    fn deliver_steps(
        self,
        worktree: Worktree,
        opening: Opening,
        pull_request: &mut PullRequest,
        checklist: Option<&str>,
        steps: Option<Vec<crate::processes::ticket::Step>>,
    ) -> Result<Reached, FailedRun> {
        let tracker = Tracker::new(opening.session.stage(), CommandLog);
        let logs = self.logs;
        let issue = self.issue;
        let started = self.started;
        let base = self.base;
        let goal = self.goal;
        let security_review = self.security_review;
        let repair_cap = self.repair_cap;
        let protected_paths = self.protected_paths;
        let child_asks = self.child_asks;
        let base_fix = self.base_fix;
        run_checked(
            CheckedRun {
                logs,
                issue,
                started,
                harness: &child_asks.harness,
                security_review: child_asks.security_review_harness.as_ref(),
                security_review_default: child_asks.security_review_default.as_ref(),
                base,
            },
            worktree,
            pull_request,
            &tracker,
            |resolution, log| Reached {
                resolution,
                log,
                ticket_lines: Vec::new(),
            },
            |merged, log| Reached {
                resolution: Resolution::PullRequest {
                    url: merged.pr.url.clone(),
                    goal: Goal::Merged,
                },
                log,
                ticket_lines: Vec::new(),
            },
            |tracker, worktree, pull_request, sessions| {
                let route = Route {
                    security_review,
                    issue,
                    base,
                    branch: worktree.branch(),
                    goal,
                    harness: &child_asks.harness,
                };
                let mut outside = InWorktree {
                    tracker,
                    issue,
                    worktree,
                    base,
                    base_fix,
                    child_asks,
                    sessions,
                    pull_request,
                    repair_cap,
                    protected_paths,
                };
                match steps {
                    Some(steps) => route.process_steps(&mut outside, &opening, steps),
                    None => route.spec_steps(&mut outside, &opening.prompt, checklist),
                }
            },
        )
    }

    /// Publish the Issue branch already produced by a Custom process. There
    /// is no opening or Security review session: the Process's Steps did
    /// the work. Carry their review outcome into readiness and Self-merge.
    /// Push, open a draft pull request when the branch has none,
    /// write the Built-with line, mark it ready, and keep it mergeable and
    /// its CI green through the Repair loop. For [`Goal::Merged`], Self-merge
    /// it, unless a protected path holds that merge. Repairs run on
    /// [`Delivery::child_asks`]'s Harness, up to [`Delivery::repair_cap`].
    /// Any failure goes through the Failed run path.
    pub fn publish(
        self,
        worktree: Worktree,
        pull_request: &mut PullRequest,
        reviews: &[SecurityOutcome],
    ) -> Result<String, FailedRun> {
        let tracker = Tracker::new(Stage::Push, CommandLog);
        let logs = self.logs;
        let issue = self.issue;
        let started = self.started;
        let base = self.base;
        let goal = self.goal;
        let repair_cap = self.repair_cap;
        let protected_paths = self.protected_paths;
        let child_asks = self.child_asks;
        let base_fix = self.base_fix;
        run_checked(
            CheckedRun {
                logs,
                issue,
                started,
                harness: &child_asks.harness,
                security_review: child_asks.security_review_harness.as_ref(),
                security_review_default: child_asks.security_review_default.as_ref(),
                base,
            },
            worktree,
            pull_request,
            &tracker,
            |url, _log| url,
            |merged, _log| merged.pr.url.clone(),
            |tracker, worktree, pull_request, sessions| {
                publish_steps(
                    &mut InWorktree {
                        tracker,
                        issue,
                        worktree,
                        base,
                        base_fix,
                        child_asks,
                        sessions,
                        pull_request,
                        repair_cap,
                        protected_paths,
                    },
                    goal,
                    reviews,
                )
            },
        )
    }
}

/// What the shared Delivery envelope checks before each session and names
/// when a failure preserves the work: the issue, when the run began, where
/// its sessions are logged, the harnesses they run on, and the Base branch.
struct CheckedRun<'a> {
    logs: &'a Logs,
    issue: &'a IssueUrl,
    started: DateTime<Utc>,
    harness: &'a Choice,
    security_review: Option<&'a Choice>,
    security_review_default: Option<&'a Choice>,
    base: &'a str,
}

/// The shared Delivery envelope: check for an untrusted edit before each
/// session, run `steps`, and on failure either accept an already-merged pull
/// request whose head includes the local work or take the Failed run path.
fn run_checked<T, S>(
    checked: CheckedRun<'_>,
    worktree: Worktree,
    pull_request: &mut PullRequest,
    tracker: &Tracker,
    on_ok: impl FnOnce(T, Option<PathBuf>) -> S,
    on_merged: impl FnOnce(&AlreadyMerged, Option<PathBuf>) -> S,
    steps: impl FnOnce(&Tracker, &Worktree, &mut PullRequest, &Sessions) -> Result<T>,
) -> Result<S, FailedRun> {
    let CheckedRun {
        logs,
        issue,
        started,
        harness,
        security_review,
        security_review_default,
        base,
    } = checked;
    pull_request.begin_delivery();
    let path = worktree.path().to_path_buf();
    let before_each = move || {
        no_untrusted_edit_since(issue, started, "Ticket run")?;
        hand_over_issue(issue, &path)
    };
    let (delivered, log) = Sessions::checked_within(
        logs,
        worktree.path(),
        harness,
        security_review,
        security_review_default,
        &before_each,
        |sessions| steps(tracker, &worktree, pull_request, sessions),
    );
    let result = match delivered {
        Ok(value) => Ok(on_ok(value, log)),
        Err(error) => {
            if let Some(merged) = error.downcast_ref::<AlreadyMerged>()
                && worktree.fully_landed_at(&merged.head).unwrap_or(false)
            {
                progress::step("the pull request was merged while Delivery was running");
                Ok(on_merged(merged, log))
            } else {
                let stopped_at = stopped_at(tracker, &error);
                let path = worktree.path().to_path_buf();
                let mut outside = OnFailure {
                    base,
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

/// [`Delivery::publish`]'s steps: push, open a draft when none exists, write
/// what it was built with, mark the pull request ready, and take it to `goal`
/// through the Repair loop. A merged goal then deletes the branch and closes
/// the issue, as a Ticket run's Self-merge does.
fn publish_steps(
    outside: &mut InWorktree<'_>,
    goal: Goal,
    reviews: &[SecurityOutcome],
) -> Result<String> {
    outside.tracker.enter(Stage::Push);
    outside.pull_request.ensure_actionable(false)?;
    outside
        .worktree
        .push_if_actionable(&mut || outside.pull_request.ensure_actionable(false))?;
    let title = GitHub::new().issue_title(outside.issue)?;
    let body = format!("Closes #{}", outside.issue.number);
    outside.pull_request.open_if_absent(&title, &body)?;
    if let Err(error) = outside
        .pull_request
        .write_built_with(&outside.child_asks.harness)
    {
        progress::warn(
            &error,
            format_args!("could not write the Built-with line in the pull request's body"),
        );
    }
    outside.tracker.enter(Stage::PullRequestReadiness);
    let pr = outside.pull_request.mark_ready(None)?;
    take_reviewed_to_goal(outside, &pr.url, goal, reviews)?;
    if goal == Goal::Merged {
        let branch = outside.worktree.branch().to_string();
        Route {
            security_review: false,
            issue: outside.issue,
            base: outside.base,
            branch: &branch,
            goal,
            harness: &outside.child_asks.harness,
        }
        .after_merge(outside, &pr);
    }
    Ok(pr.url)
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

impl Route<'_> {
    /// [`Delivery::spec_review`]'s steps, through `outside`, in order: catch up
    /// from origin, the Spec review, the push, the
    /// line saying what it was built with, checklist restoration, marking the
    /// pull request ready, taking it to the goal through the Repair loop, then for [`Goal::Merged`], the steps after
    /// the merge. Returns the pull request's URL.
    fn spec_steps<O: Outside>(
        &self,
        outside: &mut O,
        prompt: &str,
        checklist: Option<&str>,
    ) -> Result<Resolution> {
        outside.catch_up()?;
        outside.stage(Stage::SpecReview);
        outside.session(Kind::SpecReview, prompt)?;
        let review = self.review(outside)?;
        let pr = self.publish(outside, checklist)?;
        self.finish(outside, pr, self.goal, review.as_slice())
    }

    /// The file supplies the sequence. Delivery and its adjacent Self-merge
    /// share one Repair loop, just as in a Custom process.
    fn process_steps(
        &self,
        outside: &mut impl Outside,
        opening: &Opening,
        steps: Vec<crate::processes::ticket::Step>,
    ) -> Result<Resolution> {
        use crate::processes::ticket::Step;

        let mut steps = steps.into_iter().peekable();
        let mut review = None;
        let mut resolution = None;
        while let Some(step) = steps.next() {
            match step {
                Step::Implement => {
                    if let Some(early) = self.implement(outside, opening)? {
                        return Ok(early);
                    }
                }
                Step::SecurityReview => review = self.review(outside)?,
                Step::Delivery => {
                    let goal = if matches!(steps.peek(), Some(Step::SelfMerge)) {
                        steps.next();
                        self.goal
                    } else {
                        Goal::ReadyForReview
                    };
                    let pr = self.publish(outside, None)?;
                    resolution = Some(self.finish(outside, pr, goal, review.as_slice())?);
                }
                Step::SelfMerge => anyhow::bail!("self-merge must directly follow delivery"),
            }
        }
        resolution.context("ticket-run ended without delivery")
    }

    fn implement(
        &self,
        outside: &mut impl Outside,
        opening: &Opening,
    ) -> Result<Option<Resolution>> {
        let session = opening.session;
        outside.stage(session.stage());
        if let crate::implement::Completed::NotReproduced(comment_url) =
            crate::implement::run(self.issue, opening, |prompt| {
                outside.session(session.kind(), prompt)
            })?
        {
            outside.confirm_needs_info()?;
            return Ok(Some(Resolution::NotReproduced { comment_url }));
        }
        Ok(None)
    }

    fn review(&self, outside: &mut impl Outside) -> Result<Option<SecurityOutcome>> {
        self.security_review
            .then(|| {
                outside.stage(Stage::SecurityReview);
                outside.security_review(&prompt::security_review(
                    self.issue,
                    self.base,
                    self.branch,
                ))
            })
            .transpose()
    }

    fn publish(&self, outside: &mut impl Outside, checklist: Option<&str>) -> Result<Identified> {
        outside.stage(Stage::Push);
        outside.push()?;
        self.write_built_with(outside);
        outside.stage(Stage::PullRequestReadiness);
        outside.mark_pr_ready(checklist)
    }

    fn finish(
        &self,
        outside: &mut impl Outside,
        pr: Identified,
        goal: Goal,
        reviews: &[SecurityOutcome],
    ) -> Result<Resolution> {
        take_reviewed_to_goal(outside, &pr.url, goal, reviews)?;
        if goal == Goal::Merged {
            self.after_merge(outside, &pr);
        }
        Ok(Resolution::PullRequest { url: pr.url, goal })
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
/// `PolicyRefusal`, `SecurityHold` or `MergeHold` neither pushes nor converts, so the PR stays ready on the
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
    let keep_ready = crate::failed_run::is_hold(&error);
    let log = error
        .downcast_ref::<SecurityHold>()
        .and_then(SecurityHold::log)
        .map(Path::to_path_buf)
        .or(log);
    let preservation = if keep_ready {
        Preservation::Remote
    } else {
        let preserved = outside.preserve_failed_run(&reason);
        recovery::preservation_of(preserved, |line| outside.step(line))
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

/// Every review's public outcome reaches the PR. Any Security hold prevents
/// Self-merge, while readiness still includes repairing conflicts and CI.
fn take_reviewed_to_goal(
    outside: &mut impl Outside,
    pr_url: &str,
    goal: Goal,
    reviews: &[SecurityOutcome],
) -> Result<()> {
    let entry = reviews
        .iter()
        .map(SecurityOutcome::entry)
        .collect::<Vec<_>>()
        .join("\n");
    if !reviews.is_empty()
        && let Err(error) = outside.record_security_review(&entry)
    {
        outside.warn(
            &error,
            "could not write the Security review outcome in the pull request's body".to_string(),
        );
    }
    let hold = reviews.iter().find_map(SecurityOutcome::hold);
    outside.take_to_goal(
        pr_url,
        if hold.is_some() {
            Goal::ReadyForReview
        } else {
            goal
        },
    )?;
    if let Some(hold) = hold {
        outside.step(hold.to_string());
        if goal == Goal::Merged {
            return Err(hold.into());
        }
    }
    Ok(())
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
    fn confirm_needs_info(&mut self) -> Result<()>;
    /// Run the Security review given `prompt`, with its Session trace, its
    /// Resume's if it got one, if it started a session.
    fn security_review(&mut self, prompt: &str) -> Result<SecurityOutcome>;
    fn record_security_review(&mut self, entry: &str) -> Result<()>;
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
pub(crate) fn no_untrusted_edit_since(
    issue: &IssueUrl,
    started: DateTime<Utc>,
    run_name: &'static str,
) -> Result<()> {
    match trust::untrusted_edit(&GitHub::new(), issue, Some(started))? {
        Some(text) => Err(UntrustedEdit {
            number: issue.number,
            text,
            run_name,
        }
        .into()),
        None => Ok(()),
    }
}

/// Write `issue` to [`prompt::ISSUE_FILE`] in `worktree`, kept out of git,
/// for the session about to start to work from: its title, its body and only
/// the comments of Trusted accounts, as [`GitHub::issue_for_session`] gives
/// them. Written again before each session, so each has the latest.
pub(crate) fn hand_over_issue(issue: &IssueUrl, worktree: &Path) -> Result<()> {
    skills::exclude(&Git::new(worktree), &format!("/{}", prompt::ISSUE_FILE))?;
    let text = GitHub::new().issue_for_session(issue)?;
    let path = worktree.join(prompt::ISSUE_FILE);
    fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))
}

/// Issue `number`'s `text` was edited during its Run by an account that is
/// not a Trusted account, so the Run starts no further session.
#[derive(Debug)]
pub(crate) struct UntrustedEdit {
    number: u64,
    text: IssueText,
    run_name: &'static str,
}

impl std::fmt::Display for UntrustedEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "#{} {} edited since the {} started by an account that is not a Trusted account; starting no further session",
            self.number, self.text, self.run_name
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
    repair_cap: usize,
    protected_paths: &'a [String],
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
        crate::implement::confirm_needs_info(&GitHub::new().issue_labels(self.issue)?)
    }

    fn security_review(&mut self, prompt: &str) -> Result<SecurityOutcome> {
        step::Step {
            sessions: self.sessions,
            worktree: self.worktree,
            issue: self.issue,
            base: self.base,
        }
        .run(self.pull_request, prompt)
    }

    fn record_security_review(&mut self, entry: &str) -> Result<()> {
        self.pull_request.record_security_review(entry)
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
            protected_paths: self.protected_paths,
        };
        repair_loop::take_to_goal(&mut outside, self.base, goal, self.repair_cap)
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

impl InWorktree<'_> {}

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
    /// Globs that hold Self-merge when a changed path matches. Empty when
    /// this Delivery has nothing to hold.
    protected_paths: &'a [String],
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
            spec_pr: self.pull_request.spec_pr().cloned(),
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

    fn merge_hold(&mut self) -> Result<Option<String>> {
        if self.protected_paths.is_empty() {
            return Ok(None);
        }
        let spec = format!("origin/{}...HEAD", self.base);
        // `--no-renames`: include both endpoints of a move, because removing
        // a protected source still touches it. `-z`: names stay unquoted,
        // so `secrets/kéy.txt` still matches `secrets/**`.
        // Line splitting would also cut a name holding a newline in two.
        let listed = Git::new(self.worktree.path()).run_bytes(&[
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            &spec,
        ])?;
        Ok(protected_hold(
            self.protected_paths,
            listed
                .split(|byte| *byte == 0)
                .filter(|name| !name.is_empty()),
        ))
    }

    fn interrupt_requested(&mut self) -> bool {
        interrupt::requested()
    }

    fn progress(&mut self, line: String) {
        progress::step(line);
    }
}

/// The protected paths among `paths`, named in the hold a Self-merge reports.
/// None when nothing matches. Paths are raw diff bytes, so names git would
/// quote still match. `*` stays inside one path segment, `**` crosses
/// segments, and `?` is one character other than `/`.
fn protected_hold<'a>(
    patterns: &[String],
    paths: impl Iterator<Item = &'a [u8]>,
) -> Option<String> {
    let mut matched: Vec<&[u8]> = paths
        .filter(|path| {
            patterns
                .iter()
                .any(|pattern| glob_match(pattern.as_bytes(), path))
        })
        .collect();
    if matched.is_empty() {
        return None;
    }
    matched.sort_unstable();
    matched.dedup();
    let listed = matched
        .iter()
        .map(|path| String::from_utf8_lossy(path))
        .collect::<Vec<_>>()
        .join(", ");
    Some(if matched.len() == 1 {
        format!(
            "protected path {listed} holds the Self-merge; the pull request stays ready for review"
        )
    } else {
        format!(
            "protected paths {listed} hold the Self-merge; the pull request stays ready for review"
        )
    })
}

/// Whether `pattern` matches the whole of `path`.
fn glob_match(pattern: &[u8], path: &[u8]) -> bool {
    let mut pattern_at = 0;
    let mut path_at = 0;
    while pattern_at < pattern.len() {
        if pattern[pattern_at] == b'*' && pattern.get(pattern_at + 1) == Some(&b'*') {
            pattern_at += 2;
            if pattern.get(pattern_at) == Some(&b'/') {
                pattern_at += 1;
            }
            if pattern_at == pattern.len() {
                return true;
            }
            // The rest of the pattern starts at a segment boundary. `**` does
            // not match inside a name, so `**/key.txt` is not `mykey.txt`.
            let mut rest = path_at;
            loop {
                if glob_match(&pattern[pattern_at..], &path[rest..]) {
                    return true;
                }
                if rest >= path.len() {
                    return false;
                }
                while rest < path.len() && path[rest] != b'/' {
                    rest += 1;
                }
                if rest == path.len() {
                    return false;
                }
                rest += 1;
            }
        } else if pattern[pattern_at] == b'*' {
            pattern_at += 1;
            loop {
                if glob_match(&pattern[pattern_at..], &path[path_at..]) {
                    return true;
                }
                if path_at == path.len() || path[path_at] == b'/' {
                    return false;
                }
                path_at += 1;
            }
        } else if pattern[pattern_at] == b'?' {
            if path_at == path.len() || path[path_at] == b'/' {
                return false;
            }
            pattern_at += 1;
            path_at += utf8_char_len(&path[path_at..]);
        } else {
            if path_at == path.len() || pattern[pattern_at] != path[path_at] {
                return false;
            }
            pattern_at += 1;
            path_at += 1;
        }
    }
    path_at == path.len()
}

/// Bytes of the UTF-8 character at the start of `bytes`. A lone non-UTF-8
/// byte counts as one, so `?` still advances.
fn utf8_char_len(bytes: &[u8]) -> usize {
    let width = match bytes[0] {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return 1,
    };
    if bytes.len() >= width && bytes[1..width].iter().all(|byte| byte & 0xC0 == 0x80) {
        width
    } else {
        1
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
