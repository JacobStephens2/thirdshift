//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on, and only while that run is live. It is
//! made before the run's work, and ended once with how the run ended: removed
//! once its Self-merge has left the issue closed, removed without putting
//! anything back when its pull request is ready for review, and released when
//! the run failed with nothing on origin to take over. A failed run with work
//! on origin, and a failed Security fix, swap it for `agent-failed`, which the
//! Failure limit counts and the next Claim takes off. A Spec run that is only
//! waiting swaps it for `ready-for-agent`, or for `needs-triage` after an
//! untrusted edit of its Spec. A Stop instead leaves
//! the issue for triage, or explicitly releases it when asked to requeue. A
//! Custom process's Delivery removes it when the pull request is ready for
//! review, without putting `ready-for-agent` back.
//!
//! What it reads and changes of GitHub and origin, whether the run is
//! interrupted, and its progress lines all go through [`Outside`]:
//! [`OnGitHub`] does each through `gh`, `git` and the progress lines;
//! `InMemory`, in tests, from memory, recording each call.

use anyhow::{Context, Result};

use crate::branch;
use crate::git::Git;
use crate::github::GitHub;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::labels::{self, Edit, Label, Labels, READY_FOR_AGENT};
use crate::progress;
use crate::run::Goal;
use crate::security::review::{
    PrivateReportMaterial, private_report_material, private_report_material_if_configured,
};
use crate::trust::{self, IssueText};

/// The label of a Claimed issue.
pub const IN_PROGRESS: Label =
    Label::new("in-progress", "A Claim: thirdshift has taken this issue");

/// The label of an issue the factory failed on: a Failed ticket run, Failed
/// spec run or failed Custom process left work on origin, or a Security fix
/// failed, and it waits for the Day shift. It takes the place of the Claim's
/// `in-progress`. The Failure limit counts the open issues that carry it.
pub const AGENT_FAILED: Label = Label::new(
    "agent-failed",
    "thirdshift failed on this issue; it waits for the Day shift",
);

/// How the Run or the Spec run that made a Claim ended, as ending the Claim
/// needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It reached this goal.
    Reached(Goal),
    /// A bug Ticket run could not reproduce the bug, and handed it back to
    /// the reporter.
    NotReproduced,
    /// A Custom process ended successfully before Delivery. Release its
    /// Claim only if nothing is on origin, without labelling a failure.
    BeforeDelivery,
    /// It failed with its pull request left ready for review: a policy,
    /// security or merge hold held its Self-merge.
    Held,
    /// A Spec run ended only waiting: no Ticket failed or was interrupted,
    /// and every Ticket not done waits on a person or on other work.
    Waiting,
    /// It failed because the host ran out of disk space, which says nothing
    /// about the issue: the Claim is released whatever is on origin, without
    /// labelling a failure.
    OutOfDisk,
    /// It ended as a Paused Command: no route it may take had available
    /// Plan usage, which says nothing about the issue. The Claim is released
    /// whatever is on origin, without labelling a failure.
    Paused,
    /// It failed otherwise.
    Failed,
}

impl Ending {
    /// How a run that failed with `error` ends its Claim: [`Ending::Paused`]
    /// when structured Plan-usage facts paused it, [`Ending::OutOfDisk`]
    /// when the host ran out of disk space, else [`Ending::Failed`].
    pub fn failed(error: &anyhow::Error) -> Self {
        if crate::session::limits::plan_usage_paused(error).is_some() {
            Self::Paused
        } else if crate::failed_run::out_of_disk(error) {
            Self::OutOfDisk
        } else {
            Self::Failed
        }
    }
}

/// Whether an issue with `labels` carries a Claim, whatever else it is
/// labelled.
pub fn is_on(labels: &Labels) -> bool {
    labels.has(IN_PROGRESS)
}

/// The Claim a Run or a Spec run made on its issue from its Launch directory,
/// which it asks origin through when the Claim is released.
pub struct Claim<'a> {
    launch: &'a Git,
    claimed: Claimed<'a>,
    branch: String,
    /// Captured before the run's sessions: one can read a private Security
    /// report and delete it, and its failure can be posted in a comment.
    private_reports: Vec<PrivateReportMaterial>,
}

/// The comment posted in place of one that included private Security
/// review material.
const WITHHELD: &str = "thirdshift's Run on this issue ended, but how it ended is withheld: it \
                        included private Security review material. Put `ready-for-agent` back \
                        to retry.";

/// Make the Claim on `issue`, from its Launch directory `launch`: label it
/// `in-progress`, in place of `ready-for-agent` and `agent-failed` if it has
/// them, in one request that keeps its other labels, having added
/// `in-progress` to the repository if it lacks it. An issue already Claimed,
/// `in-progress` and not `ready-for-agent`, only has `agent-failed` taken
/// off, if it has that, a failure to do so a warning; with neither, no
/// request is made. A failure names the Claim as what could not be made.
pub fn make<'a>(issue: &'a IssueUrl, launch: &'a Git, branch: &str) -> Result<Claim<'a>> {
    let claimed = Claimed::make(
        &mut OnGitHub {
            launch,
            github: GitHub::new(),
            branch,
        },
        issue,
    )?;
    Ok(Claim {
        launch,
        claimed,
        branch: branch.into(),
        private_reports: vec![private_report_material_if_configured(&issue.repo())],
    })
}

impl Claim<'_> {
    /// Capture the private Security reports again, for a run whose logs were
    /// configured only after it made the Claim, before its sessions start.
    pub fn capture_private_reports(&mut self) {
        let reports = private_report_material(&self.claimed.issue.repo());
        self.private_reports.push(reports);
    }

    /// End the Claim once the Run or the Spec run that made it has ended as
    /// `ending` says:
    ///
    /// - Merged: the Claim is removed if the Self-merge left the issue
    ///   closed.
    /// - Ready for review, or held with its pull request ready: the Claim is
    ///   removed and nothing put back, open issue or not. The issue is not
    ///   Ready, so Pickup does not take it again while the pull request waits.
    /// - Not reproduced: a bug Ticket run handed reproduction back to the
    ///   reporter, so the Claim ends without making the issue Ready again,
    ///   including in a Continuation, and even after a Stop.
    /// - Failed: the Claim is released if nothing of the run is on origin to
    ///   take over, except a Security fix. Otherwise `agent-failed` takes the
    ///   place of `in-progress` in one label edit.
    /// - Out of disk: the host failed, not the issue, so the Claim is
    ///   released even with work on origin, except a Security fix, which ends
    ///   as a failed one does.
    /// - Paused: no route had Plan usage, so the Claim is released even
    ///   with work on origin, putting `ready-for-agent` back if the Claim
    ///   took it off, so a later Pickup run continues it, or `needs-triage`
    ///   after an untrusted edit, except a Security fix, which ends as a
    ///   failed one does.
    /// - Before Delivery: a successful Custom process releases its Claim if
    ///   nothing is on origin, except a Security fix, and adds no failure label.
    /// - Waiting: a Spec run only waiting hands its Spec back, putting
    ///   `ready-for-agent` back in place of `in-progress` if the Claim took it
    ///   off, so Pickup continues it once a Ticket can start. If an account
    ///   that is not a Trusted account edited the Spec's title or body since
    ///   its last `ready-for-agent` labelling by a Trusted account, it is
    ///   labelled `needs-triage` instead. A Claim that took no
    ///   `ready-for-agent` off only takes `in-progress` off.
    /// - Stopped: the issue goes to triage, keeping its Claim only while work
    ///   remains on origin, or is released even with work when asked to requeue.
    ///
    /// Then post `comment`, if given, on the issue when the ending owes one:
    /// the Recovery comment after labelling the issue `agent-failed`, except
    /// for a failed Security fix, or the waiting Spec's comment after handing
    /// it back. A released Claim, a Stop and every other ending post nothing.
    ///
    /// The run has ended as it has, so this never fails: a failure is a
    /// warning naming what to run by hand, and an interrupt doesn't stop it.
    pub fn end(self, ending: Ending, comment: Option<String>) {
        let launch = self.launch.completion();
        let outside = &mut OnGitHub {
            launch: &launch,
            github: GitHub::new().completion(),
            branch: &self.branch,
        };
        if self.claimed.end(outside, ending)
            && let Some(comment) = comment
        {
            // A prompt not to copy a private Security report is not a
            // boundary, so any stored report fragment replaces the whole
            // comment.
            let current = private_report_material_if_configured(&self.claimed.issue.repo());
            let comment = if self
                .private_reports
                .iter()
                .chain([&current])
                .any(|reports| reports.exposes(&comment))
            {
                format!("{}\n\n{WITHHELD}\n", crate::github::GENERATED)
            } else {
                comment
            };
            self.claimed.post(outside, &comment);
        }
    }

    /// Take `in-progress` off the issue, whether or not it is open, and put
    /// nothing back. A Delivery that left a pull request ready for review
    /// ends this way: the issue is not Ready, so Pickup does not take it
    /// again while that pull request is open. A failure is a warning naming
    /// what to run by hand.
    pub fn remove(self) {
        let launch = self.launch.completion();
        self.claimed.remove(&mut OnGitHub {
            launch: &launch,
            github: GitHub::new().completion(),
            branch: &self.branch,
        });
    }
}

/// What the Claim reads and changes of its issue on GitHub and of origin,
/// whether the run is interrupted, and where its progress lines go.
trait Outside {
    /// The issue's labels.
    fn labels(&mut self, issue: &IssueUrl) -> Result<Labels>;
    /// Whether the issue is open, and its labels, read together.
    fn state(&mut self, issue: &IssueUrl) -> Result<State>;
    /// Whether the issue was started on origin: an Issue branch for it,
    /// which for a Spec is its Spec branch, or a pull request from one,
    /// open, merged or closed.
    fn started(&mut self, issue: &IssueUrl) -> Result<bool>;
    /// Whether this Run's selected Issue branch or an open PR from it remains.
    fn remaining(&mut self, issue: &IssueUrl) -> Result<bool>;
    /// The first part of the issue's text an account that is not a Trusted
    /// account edited since its last `ready-for-agent` labelling by a Trusted
    /// account, if one did.
    fn untrusted_edit(&mut self, issue: &IssueUrl) -> Result<Option<IssueText>>;
    /// Make `edit`.
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    /// Post `body` as a comment on `issue`.
    fn comment(&mut self, issue: &IssueUrl, body: &str) -> Result<()>;
    /// Whether the run was interrupted.
    fn interrupted(&mut self) -> bool;
    /// Take note that a Paused Command's Claim put `ready-for-agent` back,
    /// so a later Pickup run retakes the issue once Plan usage allows.
    fn retaken_by_pickup(&mut self);
    /// The requested Stop ending, if this Command is handling a Stop.
    fn stop(&mut self) -> Option<crate::logs::stop::Ending>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
    /// Hand on `error`, then the warning `warning` saying what to do about it
    /// by hand.
    fn warn(&mut self, error: &anyhow::Error, warning: String);
}

/// Whether an issue is open, and its labels, as read together.
struct State {
    is_open: bool,
    labels: Labels,
}

/// The issue on GitHub, and origin as the Launch directory `launch` sees it.
struct OnGitHub<'a> {
    launch: &'a Git,
    github: GitHub,
    branch: &'a str,
}

impl Outside for OnGitHub<'_> {
    fn labels(&mut self, issue: &IssueUrl) -> Result<Labels> {
        self.github.issue_labels(issue)
    }

    fn state(&mut self, issue: &IssueUrl) -> Result<State> {
        let issue = self.github.issue(issue)?;
        Ok(State {
            is_open: issue.is_open,
            labels: issue.labels,
        })
    }

    fn started(&mut self, issue: &IssueUrl) -> Result<bool> {
        Ok(branch::started(self.launch, &self.github, issue)?.is_some())
    }

    fn remaining(&mut self, issue: &IssueUrl) -> Result<bool> {
        if self.launch.on_origin(self.branch)? {
            return Ok(true);
        }
        Ok(self
            .github
            .pull_requests_with_head_prefix(issue, self.branch)?
            .iter()
            .any(|pr| pr.head == self.branch && pr.state == crate::github::PrState::Open))
    }

    fn untrusted_edit(&mut self, issue: &IssueUrl) -> Result<Option<IssueText>> {
        trust::untrusted_edit_since_ready(&self.github, issue)
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply(&self.github)
    }

    fn comment(&mut self, issue: &IssueUrl, body: &str) -> Result<()> {
        self.github.comment_issue(issue, body)
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn retaken_by_pickup(&mut self) {
        crate::logs::retaken_by_pickup();
    }

    fn stop(&mut self) -> Option<crate::logs::stop::Ending> {
        crate::logs::stop_requested().map(|request| request.ending())
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }

    fn warn(&mut self, error: &anyhow::Error, warning: String) {
        progress::warn(error, format_args!("{warning}"));
    }
}

/// The Claim on an issue, with what making it changed, which is what
/// releasing it puts back.
struct Claimed<'a> {
    issue: &'a IssueUrl,
    /// Whether making it added `in-progress`: the issue was not Claimed
    /// already.
    added_in_progress: bool,
    /// Whether making it took `ready-for-agent` off the issue.
    removed_ready_for_agent: bool,
    /// A failed Security fix stays with the Day shift even without a push.
    security_fix: bool,
}

impl<'a> Claimed<'a> {
    /// [`make`], through `outside`.
    fn make(outside: &mut impl Outside, issue: &'a IssueUrl) -> Result<Claimed<'a>> {
        Claimed::label_in_progress(outside, issue)
            .with_context(|| format!("could not make the Claim on #{}", issue.number))
    }

    /// [`Claimed::make`], its failure as `gh` gave it.
    fn label_in_progress(outside: &mut impl Outside, issue: &'a IssueUrl) -> Result<Claimed<'a>> {
        let edit = Edit::of(
            issue,
            outside.labels(issue)?,
            &[READY_FOR_AGENT, AGENT_FAILED],
            &[IN_PROGRESS],
        );
        let claimed = Claimed {
            issue,
            added_in_progress: edit.puts_on(IN_PROGRESS),
            removed_ready_for_agent: edit.takes_off(READY_FOR_AGENT),
            security_fix: edit.labels().has(crate::security::fixing::SECURITY_FIX),
        };
        let removed_agent_failed = edit.takes_off(AGENT_FAILED);
        if !claimed.changed_any() {
            if removed_agent_failed {
                outside.step(format!(
                    "taking {AGENT_FAILED} off #{}, which is Claimed again",
                    issue.number
                ));
                if let Err(error) = outside.apply(&edit) {
                    outside.warn(
                        &error,
                        format!(
                            "could not take {AGENT_FAILED} off #{}, so take it off by hand: {}",
                            issue.number,
                            labels::by_hand(issue, &[AGENT_FAILED], &[])
                        ),
                    );
                }
            }
            return Ok(claimed);
        }
        let replaced = labels::joined(
            [
                (claimed.removed_ready_for_agent, READY_FOR_AGENT),
                (removed_agent_failed, AGENT_FAILED),
            ]
            .into_iter()
            .filter_map(|(removed, label)| removed.then_some(label)),
        );
        if replaced.is_empty() {
            outside.step(format!("labelling #{} {IN_PROGRESS}", issue.number));
        } else {
            outside.step(format!(
                "labelling #{} {IN_PROGRESS}, in place of {replaced}",
                issue.number
            ));
        }
        outside.apply(&edit)?;
        Ok(claimed)
    }

    /// [`Ending::NotReproduced`], through `outside`.
    fn needs_info(&self, outside: &mut impl Outside) {
        let issue = self.issue;
        let result = (|| {
            let edit = Edit::of(
                issue,
                outside.labels(issue)?,
                &[IN_PROGRESS, READY_FOR_AGENT],
                &[labels::NEEDS_INFO],
            );
            outside.apply(&edit)
        })();
        if let Err(error) = result {
            outside.warn(
                &error,
                format!(
                    "could not end the Claim on #{} after diagnosis; run: {}",
                    issue.number,
                    labels::by_hand(
                        issue,
                        &[IN_PROGRESS, READY_FOR_AGENT],
                        &[labels::NEEDS_INFO]
                    ),
                ),
            );
        }
    }

    /// [`Claim::end`], through `outside`. Returns whether the ending owes the
    /// issue a comment: it labelled the issue `agent-failed`, or tried to,
    /// other than a failed Security fix, whose details stay in its private
    /// advisory, or it handed a waiting Spec back. A released Claim owes
    /// none, so a retry loop can't fill the issue with comments.
    fn end(&self, outside: &mut impl Outside, ending: Ending) -> bool {
        if let Ending::NotReproduced = ending {
            self.needs_info(outside);
            return false;
        }
        if let Some(stop) = outside.stop() {
            self.stopped(outside, stop);
            return false;
        }
        match ending {
            Ending::Reached(Goal::Merged) => self.remove_if_closed(outside),
            Ending::Reached(Goal::ReadyForReview) | Ending::Held => self.remove(outside),
            // Pickup never takes a Security fix, so a waiting one stays with
            // the Day shift as a failed one does, its details in its advisory.
            Ending::Waiting | Ending::Failed | Ending::OutOfDisk | Ending::Paused
                if self.security_fix =>
            {
                self.label_failed(outside);
            }
            Ending::Waiting => {
                self.hand_back(outside);
                return true;
            }
            // A NotReproduced ending was dealt with above.
            Ending::NotReproduced => {}
            Ending::BeforeDelivery if self.security_fix => {}
            Ending::BeforeDelivery => {
                self.release_if_nothing_on_origin(outside);
            }
            Ending::OutOfDisk => self.release_out_of_disk(outside),
            Ending::Paused => self.release_paused(outside),
            Ending::Failed => {
                let not_released = self.release_if_nothing_on_origin(outside);
                if not_released {
                    return self.label_failed(outside);
                }
            }
        }
        false
    }

    /// Post `comment` on the issue. The run has ended as it has, so a failure
    /// is a warning, and an interrupt doesn't stop it.
    fn post(&self, outside: &mut impl Outside, comment: &str) {
        let issue = self.issue;
        outside.step(format!(
            "commenting on #{} with how its run ended",
            issue.number
        ));
        if let Err(error) = retry_if_interrupted(outside, |outside| outside.comment(issue, comment))
        {
            outside.warn(
                &error,
                format!(
                    "could not comment on #{} with how its run ended; the run's outcome is \
                     unchanged",
                    issue.number
                ),
            );
        }
    }

    /// Label the issue `agent-failed` in place of `in-progress`, in one
    /// request that keeps its other labels, having added the label to the
    /// repository if it lacks it, if the issue still carries the Claim: the
    /// Run is no longer live, and the failure waits for the Day shift. One
    /// already labelled `agent-failed` only has `in-progress` taken off, and
    /// one no longer `in-progress` is left as it is. It finishes after an
    /// interrupt, and a failure is a warning naming what to run by hand.
    /// Returns whether it labelled the issue `agent-failed`, found it so
    /// labelled, or failed trying: all but an issue left as it is.
    fn label_failed(&self, outside: &mut impl Outside) -> bool {
        let issue = self.issue;
        match retry_if_interrupted(outside, |outside| {
            let edit = Edit::of(
                issue,
                outside.labels(issue)?,
                &[IN_PROGRESS],
                &[AGENT_FAILED],
            );
            if !edit.takes_off(IN_PROGRESS) {
                return Ok(false);
            }
            outside.step(if edit.puts_on(AGENT_FAILED) {
                format!(
                    "labelling #{} {AGENT_FAILED}, in place of {IN_PROGRESS}, so it waits for \
                     the Day shift",
                    issue.number
                )
            } else {
                format!(
                    "removing {IN_PROGRESS} from issue #{}, already labelled {AGENT_FAILED}",
                    issue.number
                )
            });
            outside.apply(&edit).map(|()| true)
        }) {
            Ok(labelled) => labelled,
            Err(error) => {
                outside.warn(
                    &error,
                    format!(
                        "could not label #{} {AGENT_FAILED} in place of {IN_PROGRESS}, so do it \
                         by hand: {}",
                        issue.number,
                        labels::by_hand(issue, &[IN_PROGRESS], &[AGENT_FAILED])
                    ),
                );
                true
            }
        }
    }

    /// [`Ending::Waiting`], through `outside`: put `ready-for-agent` back on
    /// the Spec in place of `in-progress`, in one request that keeps its other
    /// labels, or `needs-triage` if an account that is not a Trusted account
    /// edited it since its last Trusted `ready-for-agent` labelling. A Claim
    /// that took no `ready-for-agent` off has none to put back, so it only
    /// takes `in-progress` off. One no longer `in-progress` is left as it is.
    /// It finishes after an interrupt, and a failure is a warning naming what
    /// to run by hand.
    fn hand_back(&self, outside: &mut impl Outside) {
        if !self.removed_ready_for_agent {
            self.remove(outside);
            return;
        }
        let issue = self.issue;
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            if let Some(text) = outside.untrusted_edit(issue)? {
                let said = format!(
                    "labelling #{} {}, in place of {IN_PROGRESS}: its Spec run is only waiting",
                    issue.number,
                    labels::NEEDS_TRIAGE
                );
                return self.leave_for_triage(outside, text, &said).map(drop);
            }
            let edit = Edit::of(
                issue,
                outside.labels(issue)?,
                &[IN_PROGRESS],
                &[READY_FOR_AGENT],
            );
            if !edit.takes_off(IN_PROGRESS) {
                return Ok(());
            }
            outside.step(format!(
                "labelling #{} {READY_FOR_AGENT}, in place of {IN_PROGRESS}: its Spec run is \
                 only waiting",
                issue.number
            ));
            outside.apply(&edit)
        }) {
            outside.warn(
                &error,
                format!(
                    "could not hand #{} back after its Spec run ended only waiting; unless an \
                     account that is not a Trusted account edited it since it was last labelled \
                     {READY_FOR_AGENT}, run: {}",
                    issue.number,
                    labels::by_hand(issue, &[IN_PROGRESS], &[READY_FOR_AGENT])
                ),
            );
        }
    }

    fn stopped(&self, outside: &mut impl Outside, ending: crate::logs::stop::Ending) {
        use crate::logs::stop::Ending;
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            let state = outside.state(self.issue)?;
            if !state.is_open {
                return self.unlabel_if_closed(outside);
            }
            let (off, on): (&[Label], &[Label]) = match ending {
                Ending::Requeue => (&[IN_PROGRESS], &[READY_FOR_AGENT]),
                Ending::Triage if outside.remaining(self.issue)? => {
                    (&[READY_FOR_AGENT], &[labels::NEEDS_TRIAGE])
                }
                Ending::Triage => (&[IN_PROGRESS, READY_FOR_AGENT], &[labels::NEEDS_TRIAGE]),
            };
            outside.apply(&Edit::of(self.issue, state.labels, off, on))
        }) {
            outside.warn(
                &error,
                format!(
                    "could not end the Stop's Claim on #{}; check its labels by hand",
                    self.issue.number
                ),
            );
        }
    }

    /// Whether making the Claim changed any label.
    fn changed_any(&self) -> bool {
        self.added_in_progress || self.removed_ready_for_agent
    }

    /// Release the Claim if the issue was never started on origin, as a
    /// Ready issue never was. The issue's labels then go back as they were
    /// before the Claim: `in-progress` comes off if the Claim added it, and
    /// `ready-for-agent` goes back if the Claim took it off, in one request
    /// that keeps the issue's other labels, those added since included when
    /// only `in-progress` comes off. An issue that is no longer
    /// `in-progress` is left as it is: someone took the Claim off meanwhile.
    /// A Claim that changed no label has nothing to put back, and makes no
    /// request. Returns true when the Claim was not released, so is left for
    /// the ending to end otherwise: the issue was started on origin, or was
    /// Claimed before this Claim. It returns false for one released, left as
    /// it is, or whose release failed.
    fn release_if_nothing_on_origin(&self, outside: &mut impl Outside) -> bool {
        if !self.changed_any() {
            return true;
        }
        match retry_if_interrupted(outside, |outside| {
            self.put_labels_back_unless_on_origin(outside)
        }) {
            Ok(stays) => stays,
            Err(error) => {
                outside.warn(
                    &error,
                    format!(
                        "could not release the Claim on #{}, so if nothing of the run is on \
                         origin, release it by hand: {}",
                        self.issue.number,
                        self.release_by_hand()
                    ),
                );
                false
            }
        }
    }

    /// [`Ending::OutOfDisk`], through `outside`: release the Claim as
    /// [`Claimed::release_if_nothing_on_origin`] does, but whatever is on
    /// origin, which stays there for the next run to continue. It finishes
    /// after an interrupt, and a failure is a warning naming what to run by
    /// hand.
    fn release_out_of_disk(&self, outside: &mut impl Outside) {
        if !self.changed_any() {
            return;
        }
        let why = format!(
            ", without {AGENT_FAILED}: {}",
            crate::failed_run::OUT_OF_DISK
        );
        if let Err(error) =
            retry_if_interrupted(outside, |outside| self.put_labels_back(outside, &why))
        {
            outside.warn(
                &error,
                format!(
                    "could not release the Claim on #{} after the host ran out of disk space, so \
                     release it by hand: {}",
                    self.issue.number,
                    self.release_by_hand()
                ),
            );
        }
    }

    /// Label the issue `needs-triage` in place of `in-progress` and any
    /// `ready-for-agent`, in one request that keeps its other labels, saying
    /// `said` and that its `text` was edited by an account that is not a
    /// Trusted account since it was last labelled `ready-for-agent`. One no
    /// longer `in-progress` is left as it is. Returns whether it labelled it.
    fn leave_for_triage(
        &self,
        outside: &mut impl Outside,
        text: IssueText,
        said: &str,
    ) -> Result<bool> {
        let issue = self.issue;
        let edit = Edit::of(
            issue,
            outside.labels(issue)?,
            &[IN_PROGRESS, READY_FOR_AGENT],
            &[labels::NEEDS_TRIAGE],
        );
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(false);
        }
        outside.step(format!(
            "{said}, but its {text} was edited by an account that is not a Trusted account \
             since it was last labelled {READY_FOR_AGENT}"
        ));
        outside.apply(&edit).map(|()| true)
    }

    /// [`Ending::Paused`], through `outside`: release the Claim whatever is
    /// on origin, which stays there for the next run to continue, putting
    /// `ready-for-agent` back if making it took that off, unless an account
    /// that is not a Trusted account edited the issue since its last
    /// Trusted `ready-for-agent` labelling, which leaves `needs-triage`. A
    /// Claim that took no `ready-for-agent` off only takes `in-progress` off
    /// if it added that. It finishes after an interrupt, and a failure is a
    /// warning naming what to run by hand.
    fn release_paused(&self, outside: &mut impl Outside) {
        if !self.changed_any() {
            return;
        }
        let issue = self.issue;
        let why = format!(
            ", without {AGENT_FAILED}: {}",
            crate::session::limits::NO_ROUTE
        );
        let restored = retry_if_interrupted(outside, |outside| {
            if !self.removed_ready_for_agent {
                return self.put_labels_back(outside, &why).map(|()| false);
            }
            if let Some(text) = outside.untrusted_edit(issue)? {
                let said = format!(
                    "releasing the Claim on #{} as {}{why}",
                    issue.number,
                    labels::NEEDS_TRIAGE
                );
                return self.leave_for_triage(outside, text, &said).map(|_| false);
            }
            if !is_on(&outside.labels(issue)?) {
                return Ok(false);
            }
            self.put_labels_back(outside, &why).map(|()| true)
        });
        match restored {
            Ok(true) => outside.retaken_by_pickup(),
            Ok(false) => {}
            Err(error) => outside.warn(
                &error,
                format!(
                    "could not release the Claim on #{} after its Command paused, so unless an \
                     account that is not a Trusted account edited it since it was last labelled \
                     {READY_FOR_AGENT}, release it by hand: {}",
                    issue.number,
                    self.release_by_hand()
                ),
            ),
        }
    }

    /// [`Claimed::release_if_nothing_on_origin`], its failure as `git` or `gh`
    /// gave it.
    fn put_labels_back_unless_on_origin(&self, outside: &mut impl Outside) -> Result<bool> {
        if outside.started(self.issue)? {
            return Ok(true);
        }
        self.put_labels_back(outside, "").map(|()| false)
    }

    /// Put the issue's labels back as they were before the Claim, saying so
    /// with `why` after the release line, unless it is no longer `in-progress`.
    fn put_labels_back(&self, outside: &mut impl Outside, why: &str) -> Result<()> {
        let issue = self.issue;
        let (off, on) = self.release_labels();
        let edit = Edit::of(issue, outside.labels(issue)?, off, on);
        if !is_on(edit.labels()) {
            return Ok(());
        }
        let number = issue.number;
        outside.step(if !self.removed_ready_for_agent {
            format!("releasing the Claim on #{number}: removing {IN_PROGRESS}{why}")
        } else if self.added_in_progress {
            format!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT}, \
                 in place of {IN_PROGRESS}{why}"
            )
        } else {
            format!("releasing the Claim on #{number}: labelling it {READY_FOR_AGENT} again{why}")
        });
        outside.apply(&edit)
    }

    /// The labels releasing the Claim takes off and puts on, in that order:
    /// `in-progress` off if making it added that, and `ready-for-agent` on if
    /// making it took that off.
    fn release_labels(&self) -> (&'static [Label], &'static [Label]) {
        let off: &[Label] = if self.added_in_progress {
            &[IN_PROGRESS]
        } else {
            &[]
        };
        let on: &[Label] = if self.removed_ready_for_agent {
            &[READY_FOR_AGENT]
        } else {
            &[]
        };
        (off, on)
    }

    /// Take `in-progress` off, open issue or not, and put nothing on. An issue
    /// that is no longer `in-progress` is left as it is.
    fn remove(&self, outside: &mut impl Outside) {
        if let Err(error) = retry_if_interrupted(outside, |outside| self.take_off(outside)) {
            outside.warn(
                &error,
                format!(
                    "could not remove {IN_PROGRESS} from issue #{}, so remove it by hand: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &[IN_PROGRESS], &[])
                ),
            );
        }
    }

    /// [`Claimed::remove`], its failure as `gh` gave it.
    fn take_off(&self, outside: &mut impl Outside) -> Result<()> {
        let edit = Edit::of(self.issue, outside.labels(self.issue)?, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(());
        }
        outside.step(format!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        outside.apply(&edit)
    }

    /// Remove the Claim if the Self-merge has left its issue closed:
    /// `in-progress` comes off, in one request that keeps the issue's other
    /// labels. An issue still open, as when closing it failed, keeps it, and
    /// one no longer `in-progress` is left as it is.
    fn remove_if_closed(&self, outside: &mut impl Outside) {
        if let Err(error) = retry_if_interrupted(outside, |outside| self.unlabel_if_closed(outside))
        {
            outside.warn(
                &error,
                format!(
                    "could not remove {IN_PROGRESS} from issue #{}, so remove it by hand: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &[IN_PROGRESS], &[])
                ),
            );
        }
    }

    /// [`Claimed::remove_if_closed`], its failure as `gh` gave it.
    fn unlabel_if_closed(&self, outside: &mut impl Outside) -> Result<()> {
        let state = outside.state(self.issue)?;
        if state.is_open {
            return Ok(());
        }
        let edit = Edit::of(self.issue, state.labels, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(());
        }
        outside.step(format!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        outside.apply(&edit)
    }

    /// The commands that release the Claim by hand.
    fn release_by_hand(&self) -> String {
        let (off, on) = self.release_labels();
        labels::by_hand(self.issue, off, on)
    }
}

/// Run `step` through `outside`, and once more if it failed with the run
/// interrupted: Ctrl-C in a terminal also kills the git or gh the step was
/// running.
fn retry_if_interrupted<O: Outside, T>(
    outside: &mut O,
    step: impl Fn(&mut O) -> Result<T>,
) -> Result<T> {
    match step(outside) {
        Err(_) if outside.interrupted() => step(outside),
        done => done,
    }
}

#[cfg(test)]
mod in_memory {
    use anyhow::{Result, bail};

    use super::{Outside, State};
    use crate::issue::IssueUrl;
    use crate::labels::{Edit, Labels};
    use crate::trust::IssueText;

    /// A read or a change of [`Outside`] that can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fails {
        Labels,
        State,
        Started,
        Trust,
        Edit,
        Comment,
    }

    /// A call the Claim made, in the order it made it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Call {
        /// It read the issue's labels.
        Labels,
        /// It read whether the issue is open, and its labels.
        State,
        /// It asked whether the issue was started on origin.
        Started,
        /// It asked whether an untrusted account edited the issue since its
        /// last Trusted `ready-for-agent` labelling.
        Trust,
        /// It applied an Edit, taking `off` off the issue and putting `on`
        /// on it, which leaves it with `labels`.
        Edit {
            off: Vec<&'static str>,
            on: Vec<&'static str>,
            labels: Vec<String>,
        },
        /// It posted this comment on the issue, or tried to.
        Comment(String),
        /// It asked whether the run was interrupted.
        Interrupted,
        /// It handed on this progress line.
        Step(String),
        /// It handed on this error, then this warning.
        Warn { error: String, warning: String },
    }

    /// An issue in memory: its labels, whether it is open and was started
    /// on origin, whether the run was interrupted, and which reads or edits
    /// fail, and how many times more.
    #[derive(Default)]
    pub struct InMemory {
        /// The issue's labels, as spelled, which an Edit made changes.
        pub labels: Vec<String>,
        pub open: bool,
        pub started: bool,
        pub interrupted: bool,
        pub stop: Option<crate::logs::stop::Ending>,
        /// The issue's text an untrusted account edited since its last
        /// Trusted `ready-for-agent` labelling, if any.
        pub untrusted: Option<IssueText>,
        /// Each read or edit that fails, with how many times more it does.
        failing: Vec<(Fails, usize)>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
        /// Whether a Paused Command's Claim noted that Pickup retakes it.
        pub retaken_by_pickup: bool,
    }

    impl InMemory {
        /// An open issue labelled `labels`, not started on origin.
        pub fn labelled(labels: &[&str]) -> Self {
            InMemory {
                labels: labels.iter().map(|label| label.to_string()).collect(),
                open: true,
                ..InMemory::default()
            }
        }

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
        fn fail_if(&mut self, what: Fails) -> Result<()> {
            match self
                .failing
                .iter_mut()
                .find(|(failing, _)| *failing == what)
            {
                Some((_, times)) if *times > 0 => {
                    *times -= 1;
                    bail!("gh: {what:?} failed: HTTP 502")
                }
                _ => Ok(()),
            }
        }

        fn read_labels(&self) -> Labels {
            self.labels.iter().map(String::as_str).collect()
        }
    }

    impl Outside for InMemory {
        fn labels(&mut self, _: &IssueUrl) -> Result<Labels> {
            self.calls.push(Call::Labels);
            self.fail_if(Fails::Labels)?;
            Ok(self.read_labels())
        }

        fn state(&mut self, _: &IssueUrl) -> Result<State> {
            self.calls.push(Call::State);
            self.fail_if(Fails::State)?;
            Ok(State {
                is_open: self.open,
                labels: self.read_labels(),
            })
        }

        fn started(&mut self, _: &IssueUrl) -> Result<bool> {
            self.calls.push(Call::Started);
            self.fail_if(Fails::Started)?;
            Ok(self.started)
        }

        fn remaining(&mut self, issue: &IssueUrl) -> Result<bool> {
            self.started(issue)
        }

        fn untrusted_edit(&mut self, _: &IssueUrl) -> Result<Option<IssueText>> {
            self.calls.push(Call::Trust);
            self.fail_if(Fails::Trust)?;
            Ok(self.untrusted)
        }

        fn apply(&mut self, edit: &Edit) -> Result<()> {
            let (off, on) = edit.changes();
            let after = edit.labels_after();
            self.calls.push(Call::Edit {
                off: off.iter().map(|label| label.name()).collect(),
                on: on.iter().map(|label| label.name()).collect(),
                labels: after.names().map(String::from).collect(),
            });
            self.fail_if(Fails::Edit)?;
            self.labels = after.names().map(String::from).collect();
            Ok(())
        }

        fn comment(&mut self, _: &IssueUrl, body: &str) -> Result<()> {
            self.calls.push(Call::Comment(body.to_string()));
            self.fail_if(Fails::Comment)
        }

        fn interrupted(&mut self) -> bool {
            self.calls.push(Call::Interrupted);
            self.interrupted
        }

        fn stop(&mut self) -> Option<crate::logs::stop::Ending> {
            self.stop
        }

        fn retaken_by_pickup(&mut self) {
            self.retaken_by_pickup = true;
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
}

#[cfg(test)]
mod tests {
    use super::in_memory::{Call, Fails, InMemory};
    use super::*;

    fn seven() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    fn edit(off: &[&'static str], on: &[&'static str], labels: &[&str]) -> Call {
        Call::Edit {
            off: off.to_vec(),
            on: on.to_vec(),
            labels: labels.iter().map(|label| label.to_string()).collect(),
        }
    }

    fn labels(issue: &InMemory) -> Vec<&str> {
        issue.labels.iter().map(String::as_str).collect()
    }

    /// The Claim made on `issue`, with the calls making it made forgotten.
    fn claimed<'a>(issue: &mut InMemory, url: &'a IssueUrl) -> Claimed<'a> {
        let claimed = Claimed::make(issue, url).unwrap();
        issue.calls.clear();
        claimed
    }

    const RELEASE_BY_HAND: &str = "could not release the Claim on #7, so if nothing of the run \
         is on origin, release it by hand: \
         gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress && \
         gh api --method POST repos/acme/widgets/issues/7/labels -f 'labels[]=ready-for-agent'";

    #[test]
    fn a_stop_leaves_the_issue_for_triage_keeping_the_claim_only_with_work_on_origin() {
        for started in [false, true] {
            for before in [&["bug", "ready-for-agent"][..], &["bug", "in-progress"][..]] {
                let url = seven();
                let mut issue = InMemory::labelled(before);
                let claimed = claimed(&mut issue, &url);
                issue.started = started;
                issue.stop = Some(crate::logs::stop::Ending::Triage);
                issue.interrupted = true;
                claimed.end(&mut issue, Ending::Failed);
                assert_eq!(
                    labels(&issue),
                    if started {
                        vec!["bug", "in-progress", "needs-triage"]
                    } else {
                        vec!["bug", "needs-triage"]
                    }
                );
            }
        }
    }

    #[test]
    fn a_requeue_stop_releases_even_an_existing_claim_with_work_on_origin() {
        for started in [false, true] {
            let url = seven();
            let mut issue = InMemory::labelled(&["bug", "in-progress"]);
            let claimed = claimed(&mut issue, &url);
            issue.started = started;
            issue.stop = Some(crate::logs::stop::Ending::Requeue);
            claimed.end(&mut issue, Ending::Failed);
            assert_eq!(labels(&issue), ["bug", "ready-for-agent"]);
        }
    }

    #[test]
    fn a_stop_does_not_requeue_an_issue_already_closed_by_self_merge() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.stop = Some(crate::logs::stop::Ending::Requeue);
        claimed.end(&mut issue, Ending::Reached(Goal::Merged));
        assert_eq!(labels(&issue), ["bug"]);
    }

    #[test]
    fn the_claim_is_made_in_place_of_ready_for_agent_keeping_the_other_labels() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent", "architecture"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent"),
                edit(
                    &["ready-for-agent"],
                    &["in-progress"],
                    &["bug", "architecture", "in-progress"]
                ),
            ]
        );
    }

    #[test]
    fn the_claim_takes_agent_failed_off_with_ready_for_agent() {
        let url = seven();
        let mut issue = InMemory::labelled(&["agent-failed", "bug", "ready-for-agent"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent and agent-failed"),
                edit(
                    &["ready-for-agent", "agent-failed"],
                    &["in-progress"],
                    &["bug", "in-progress"]
                ),
            ]
        );
    }

    #[test]
    fn a_claim_on_an_issue_already_claimed_takes_agent_failed_off() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "Agent-Failed", "bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("taking agent-failed off #7, which is Claimed again"),
                edit(&["agent-failed"], &[], &["in-progress", "bug"]),
            ]
        );
    }

    #[test]
    fn a_claim_that_cannot_take_agent_failed_off_warns_and_is_still_made() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "agent-failed"]).failing(Fails::Edit);

        let made = Claimed::make(&mut issue, &url);

        assert!(made.is_ok());
        assert_eq!(
            issue.calls.last(),
            Some(&Call::Warn {
                error: "gh: Edit failed: HTTP 502".to_string(),
                warning: "could not take agent-failed off #7, so take it off by hand: \
                     gh api --method DELETE repos/acme/widgets/issues/7/labels/agent-failed"
                    .to_string(),
            })
        );
    }

    #[test]
    fn an_issue_with_no_ready_for_agent_is_still_claimed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress"),
                edit(&[], &["in-progress"], &["bug", "in-progress"]),
            ]
        );
    }

    #[test]
    fn an_issue_already_claimed_is_left_as_it_is_with_no_request() {
        let url = seven();
        let mut issue = InMemory::labelled(&["In-Progress", "bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(issue.calls, [Call::Labels]);
    }

    #[test]
    fn a_claimed_issue_marked_ready_again_loses_ready_for_agent_and_stays_claimed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "bug", "ready-for-agent"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent"),
                edit(&["ready-for-agent"], &[], &["in-progress", "bug"]),
            ]
        );
    }

    #[test]
    fn a_claim_that_cannot_be_made_names_the_claim_and_the_cause() {
        for failing in [Fails::Labels, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]).failing(failing);

            let error = Claimed::make(&mut issue, &url).err().unwrap();

            assert_eq!(
                format!("{error:#}"),
                format!("could not make the Claim on #7: gh: {failing:?} failed: HTTP 502")
            );
            assert_eq!(labels(&issue), ["ready-for-agent"]);
        }
    }

    #[test]
    fn a_merged_run_removes_the_claim_from_its_closed_issue() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(
            issue.calls,
            [
                Call::State,
                step("removing in-progress from issue #7"),
                edit(&["in-progress"], &[], &["bug"]),
            ]
        );
        assert_eq!(labels(&issue), ["bug"]);
    }

    #[test]
    fn a_merged_run_whose_issue_is_still_open_keeps_the_claim() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(issue.calls, [Call::State]);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_merged_run_leaves_an_issue_no_longer_in_progress_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(issue.calls, [Call::State]);
    }

    #[test]
    fn a_claim_that_cannot_be_removed_is_a_warning_naming_the_command_to_run_by_hand() {
        for failing in [Fails::State, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue.open = false;
            issue = issue.failing(failing);

            claimed.end(&mut issue, Ending::Reached(Goal::Merged));

            assert_eq!(
                issue.calls.last(),
                Some(&Call::Warn {
                    error: format!("gh: {failing:?} failed: HTTP 502"),
                    warning: "could not remove in-progress from issue #7, so remove it by hand: \
                         gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress"
                        .to_string(),
                }),
                "{failing:?}"
            );
            assert_eq!(labels(&issue), ["in-progress"]);
        }
    }

    /// The lines and the Edit that take `in-progress` off #7 and put
    /// nothing back, leaving it with `labels`.
    fn removing(labels: &[&str]) -> [Call; 3] {
        [
            Call::Labels,
            step("removing in-progress from issue #7"),
            edit(&["in-progress"], &[], labels),
        ]
    }

    #[test]
    fn a_run_ready_for_review_removes_the_claim_and_puts_nothing_back() {
        for open in [true, false] {
            let url = seven();
            let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue.started = true;
            issue.open = open;

            claimed.end(&mut issue, Ending::Reached(Goal::ReadyForReview));

            assert_eq!(issue.calls, removing(&["bug"]), "open: {open}");
            assert_eq!(labels(&issue), ["bug"], "open: {open}");
        }
    }

    #[test]
    fn a_run_ready_for_review_whose_claim_someone_took_off_makes_no_edit() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Reached(Goal::ReadyForReview));

        assert_eq!(issue.calls, [Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn a_claim_that_cannot_be_removed_at_ready_for_review_is_tried_again_after_an_interrupt_then_warned_of()
     {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.interrupted = true;
        issue = issue.failing(Fails::Edit);

        claimed.end(&mut issue, Ending::Reached(Goal::ReadyForReview));

        let mut expected: Vec<Call> = removing(&[]).into();
        expected.push(Call::Interrupted);
        expected.extend(removing(&[]));
        expected.push(Call::Warn {
            error: "gh: Edit failed: HTTP 502".to_string(),
            warning: "could not remove in-progress from issue #7, so remove it by hand: \
                 gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress"
                .to_string(),
        });
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    /// The lines and the Edit that label #7 `agent-failed` in place of
    /// `in-progress`, leaving it with `labels`.
    fn labelling_failed(labels: &[&str]) -> [Call; 3] {
        [
            Call::Labels,
            step(
                "labelling #7 agent-failed, in place of in-progress, so it waits for the Day shift",
            ),
            edit(&["in-progress"], &["agent-failed"], labels),
        ]
    }

    #[test]
    fn a_failed_run_started_on_origin_swaps_the_claim_for_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::Failed);

        let mut expected = vec![Call::Started];
        expected.extend(labelling_failed(&["bug", "agent-failed"]));
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["bug", "agent-failed"]);
    }

    #[test]
    fn a_run_the_host_ran_out_of_disk_under_is_released_even_started_on_origin_and_owes_no_comment()
    {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        let owed = claimed.end(&mut issue, Ending::OutOfDisk);

        assert!(!owed);
        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step(
                    "releasing the Claim on #7: labelling it ready-for-agent, in place of \
                     in-progress, without agent-failed: the host ran out of disk space; free disk \
                     space on it, then retry the Command"
                ),
                edit(
                    &["in-progress"],
                    &["ready-for-agent"],
                    &["bug", "ready-for-agent"]
                ),
            ]
        );
        assert_eq!(labels(&issue), ["bug", "ready-for-agent"]);
    }

    #[test]
    fn a_paused_command_releases_its_claim_even_started_on_origin_for_pickup_to_retake() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        let owed = claimed.end(&mut issue, Ending::Paused);

        assert!(!owed);
        assert_eq!(
            issue.calls,
            [
                Call::Trust,
                Call::Labels,
                Call::Labels,
                step(
                    "releasing the Claim on #7: labelling it ready-for-agent, in place of \
                     in-progress, without agent-failed: no allowed route has available Plan usage"
                ),
                edit(
                    &["in-progress"],
                    &["ready-for-agent"],
                    &["bug", "ready-for-agent"]
                ),
            ]
        );
        assert_eq!(labels(&issue), ["bug", "ready-for-agent"]);
        assert!(issue.retaken_by_pickup);
    }

    #[test]
    fn a_paused_command_after_an_untrusted_edit_leaves_its_issue_for_triage() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.untrusted = Some(IssueText::Body);

        claimed.end(&mut issue, Ending::Paused);

        assert_eq!(labels(&issue), ["bug", "needs-triage"]);
        assert!(!issue.retaken_by_pickup);
        assert!(
            !issue
                .calls
                .iter()
                .any(|call| matches!(call, Call::Comment(_)))
        );
    }

    #[test]
    fn a_paused_command_that_took_no_ready_for_agent_off_only_removes_its_claim() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Paused);

        assert_eq!(labels(&issue), ["bug"]);
        assert!(!issue.retaken_by_pickup);
    }

    #[test]
    fn a_paused_security_fix_still_waits_for_the_day_shift() {
        let url = seven();
        let mut issue = InMemory::labelled(&["security-fix", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Paused);

        assert_eq!(labels(&issue), ["security-fix", "agent-failed"]);
        assert!(!issue.retaken_by_pickup);
    }

    #[test]
    fn only_a_structured_plan_usage_pause_ends_a_claim_as_paused() {
        let paused = anyhow::Error::new(crate::session::limits::PlanUsagePaused::new(
            crate::session::limits::Limit::default(),
        ))
        .context("implement session failed");
        assert_eq!(Ending::failed(&paused), Ending::Paused);
        // Agent-writable text saying the same is an ordinary failure.
        let prose = anyhow::anyhow!(
            "no allowed route has available Plan usage; run the Command again to continue once \
             Plan usage allows"
        );
        assert_eq!(Ending::failed(&prose), Ending::Failed);
    }

    #[test]
    fn a_security_fix_the_host_ran_out_of_disk_under_still_waits_for_the_day_shift() {
        let url = seven();
        let mut issue = InMemory::labelled(&["security-fix", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::OutOfDisk);

        assert_eq!(labels(&issue), ["security-fix", "agent-failed"]);
    }

    #[test]
    fn a_failed_security_fix_swaps_its_claim_for_agent_failed_with_nothing_on_origin() {
        let url = seven();
        let mut issue = InMemory::labelled(&["security-fix", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            labelling_failed(&["security-fix", "agent-failed"])
        );
    }

    #[test]
    fn a_waiting_security_fix_spec_is_swapped_for_agent_failed_and_owes_no_comment() {
        let url = seven();
        let mut issue = InMemory::labelled(&["security-fix", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        let owed = claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(labels(&issue), ["security-fix", "agent-failed"]);
        assert!(!owed);
    }

    #[test]
    fn a_run_held_with_its_pull_request_ready_removes_the_claim_and_adds_no_label() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::Held);

        assert_eq!(issue.calls, removing(&[]));
        assert_eq!(labels(&issue), [] as [&str; 0]);
    }

    #[test]
    fn a_bug_not_reproduced_ends_the_claim_with_needs_info_and_no_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::NotReproduced);

        assert_eq!(labels(&issue), ["bug", "needs-info"]);
    }

    #[test]
    fn a_bug_not_reproduced_ends_the_claim_with_needs_info_even_after_a_stop() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.stop = Some(crate::logs::stop::Ending::Triage);

        claimed.end(&mut issue, Ending::NotReproduced);

        assert_eq!(labels(&issue), ["bug", "needs-info"]);
    }

    #[test]
    fn a_stopped_run_with_work_on_origin_is_not_labelled_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.stop = Some(crate::logs::stop::Ending::Triage);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(labels(&issue), ["in-progress", "needs-triage"]);
    }

    #[test]
    fn a_waiting_spec_run_puts_ready_for_agent_back_in_place_of_in_progress() {
        let url = seven();
        let mut issue = InMemory::labelled(&["enhancement", "ready-for-agent", "agent-failed"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(
            issue.calls,
            [
                Call::Trust,
                Call::Labels,
                step(
                    "labelling #7 ready-for-agent, in place of in-progress: its Spec run is only waiting"
                ),
                edit(
                    &["in-progress"],
                    &["ready-for-agent"],
                    &["enhancement", "ready-for-agent"]
                ),
            ]
        );
        assert_eq!(labels(&issue), ["enhancement", "ready-for-agent"]);
    }

    #[test]
    fn a_waiting_spec_run_edited_by_an_untrusted_account_is_labelled_needs_triage() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.untrusted = Some(IssueText::Title);

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(
            issue.calls,
            [
                Call::Trust,
                Call::Labels,
                step(
                    "labelling #7 needs-triage, in place of in-progress: its Spec run is only \
                     waiting, but its title was edited by an account that is not a Trusted \
                     account since it was last labelled ready-for-agent"
                ),
                edit(&["in-progress"], &["needs-triage"], &["needs-triage"]),
            ]
        );
    }

    #[test]
    fn a_waiting_spec_run_on_a_spec_never_labelled_ready_only_takes_in_progress_off() {
        let url = seven();
        let mut issue = InMemory::labelled(&["enhancement"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(issue.calls, removing(&["enhancement"]));
        assert_eq!(labels(&issue), ["enhancement"]);
    }

    #[test]
    fn a_waiting_spec_run_whose_spec_lost_its_claim_meanwhile_is_left_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(issue.calls, [Call::Trust, Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn a_waiting_spec_run_that_cannot_check_trust_keeps_its_claim_and_warns() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.interrupted = true;
        issue = issue.failing(Fails::Trust);

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(
            issue.calls,
            [
                Call::Trust,
                Call::Interrupted,
                Call::Trust,
                Call::Warn {
                    error: "gh: Trust failed: HTTP 502".to_string(),
                    warning: "could not hand #7 back after its Spec run ended only waiting; \
                         unless an account that is not a Trusted account edited it since it was \
                         last labelled ready-for-agent, run: gh api --method DELETE \
                         repos/acme/widgets/issues/7/labels/in-progress && gh api --method POST \
                         repos/acme/widgets/issues/7/labels -f 'labels[]=ready-for-agent'"
                        .to_string(),
                },
            ]
        );
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_stopped_waiting_spec_run_goes_to_triage() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.stop = Some(crate::logs::stop::Ending::Triage);

        claimed.end(&mut issue, Ending::Waiting);

        assert_eq!(labels(&issue), ["in-progress", "needs-triage"]);
    }

    #[test]
    fn a_failed_run_on_an_issue_already_labelled_agent_failed_only_takes_in_progress_off() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels.push("Agent-Failed".to_string());

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("removing in-progress from issue #7, already labelled agent-failed"),
                edit(&["in-progress"], &[], &["Agent-Failed"]),
            ]
        );
        assert_eq!(labels(&issue), ["Agent-Failed"]);
    }

    #[test]
    fn a_failed_run_whose_issue_lost_its_claim_meanwhile_is_not_labelled_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, [Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn labelling_agent_failed_is_tried_once_more_after_an_interrupt_then_warned_of() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.interrupted = true;
        issue = issue.failing(Fails::Edit);

        claimed.end(&mut issue, Ending::Failed);

        let labelling = labelling_failed(&["agent-failed"]);
        let mut expected = vec![Call::Started];
        expected.extend(labelling.clone());
        expected.push(Call::Interrupted);
        expected.extend(labelling);
        expected.push(Call::Warn {
            error: "gh: Edit failed: HTTP 502".to_string(),
            warning: "could not label #7 agent-failed in place of in-progress, so do it by \
                 hand: gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress && \
                 gh api --method POST repos/acme/widgets/issues/7/labels -f 'labels[]=agent-failed'"
                .to_string(),
        });
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_failed_run_with_nothing_on_origin_puts_the_labels_back_as_they_were() {
        for (before, line, off, on) in [
            (
                &["bug", "ready-for-agent", "architecture"][..],
                "labelling it ready-for-agent, in place of in-progress",
                &["in-progress"][..],
                &["ready-for-agent"][..],
            ),
            (
                &["bug"][..],
                "removing in-progress",
                &["in-progress"][..],
                &[][..],
            ),
            (
                &["in-progress", "ready-for-agent"][..],
                "labelling it ready-for-agent again",
                &[][..],
                &["ready-for-agent"][..],
            ),
        ] {
            let url = seven();
            let mut issue = InMemory::labelled(before);
            let claimed = claimed(&mut issue, &url);

            claimed.end(&mut issue, Ending::Failed);

            let after: Vec<&str> = before
                .iter()
                .copied()
                .filter(|label| *label != "ready-for-agent")
                .chain(on.iter().copied())
                .collect();
            assert_eq!(
                issue.calls,
                [
                    Call::Started,
                    Call::Labels,
                    step(&format!("releasing the Claim on #7: {line}")),
                    edit(off, on, &after),
                ],
                "{before:?}"
            );
            assert_eq!(labels(&issue), after, "{before:?}");
        }
    }

    #[test]
    fn a_released_claim_keeps_a_label_the_issue_was_given_during_the_run() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["in-progress".to_string(), "needs-info".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(labels(&issue), ["needs-info", "ready-for-agent"]);
    }

    #[test]
    fn an_issue_whose_claim_someone_took_off_meanwhile_is_left_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, [Call::Started, Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn a_claim_that_changed_no_label_is_swapped_for_agent_failed_when_the_run_fails() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, labelling_failed(&["agent-failed"]));
    }

    #[test]
    fn a_claim_that_cannot_be_released_is_a_warning_naming_the_commands_to_run_by_hand() {
        for failing in [Fails::Started, Fails::Labels, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue = issue.failing(failing);

            claimed.end(&mut issue, Ending::Failed);

            assert_eq!(
                issue.calls.last(),
                Some(&Call::Warn {
                    error: format!("gh: {failing:?} failed: HTTP 502"),
                    warning: RELEASE_BY_HAND.to_string(),
                }),
                "{failing:?}"
            );
            assert_eq!(labels(&issue), ["in-progress"], "{failing:?}");
        }
    }

    #[test]
    fn a_release_that_fails_while_interrupted_is_tried_once_more() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.interrupted = true;
        issue = issue.failing_times(Fails::Edit, 1);

        claimed.end(&mut issue, Ending::Failed);

        let released = || {
            [
                Call::Started,
                Call::Labels,
                step(
                    "releasing the Claim on #7: labelling it ready-for-agent, in place of in-progress",
                ),
                edit(&["in-progress"], &["ready-for-agent"], &["ready-for-agent"]),
            ]
        };
        let mut expected: Vec<Call> = released().into();
        expected.push(Call::Interrupted);
        expected.extend(released());
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["ready-for-agent"]);
    }

    #[test]
    fn a_removal_that_fails_while_interrupted_is_tried_once_more_then_warned_of() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.interrupted = true;
        issue = issue.failing(Fails::State);

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(
            issue.calls[..3],
            [Call::State, Call::Interrupted, Call::State]
        );
        assert!(matches!(issue.calls[3], Call::Warn { .. }));
        assert_eq!(issue.calls.len(), 4);
    }

    #[test]
    fn a_failure_with_no_interrupt_is_not_tried_again() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue = issue.failing_times(Fails::Started, 1);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            [
                Call::Started,
                Call::Interrupted,
                Call::Warn {
                    error: "gh: Started failed: HTTP 502".to_string(),
                    warning: RELEASE_BY_HAND.to_string(),
                },
            ]
        );
    }

    /// Whether ending a Claim made on an issue labelled `before` owes it a
    /// comment, when the run started on origin `started`, a Stop asked for
    /// `stop`, and the run ended `ending`.
    fn owes_comment(
        before: &[&str],
        started: bool,
        stop: Option<crate::logs::stop::Ending>,
        ending: Ending,
    ) -> bool {
        let url = seven();
        let mut issue = InMemory::labelled(before);
        let claimed = claimed(&mut issue, &url);
        issue.started = started;
        issue.stop = stop;
        claimed.end(&mut issue, ending)
    }

    #[test]
    fn only_an_agent_failed_ending_other_than_a_security_fix_and_a_waiting_one_owe_a_comment() {
        use crate::logs::stop::Ending::Triage;
        let ready = &["ready-for-agent"][..];
        for (before, started, stop, ending, owed) in [
            (ready, true, None, Ending::Failed, true),
            (&["in-progress"][..], false, None, Ending::Failed, true),
            (ready, false, None, Ending::Failed, false),
            (
                &["security-fix", "ready-for-agent"][..],
                true,
                None,
                Ending::Failed,
                false,
            ),
            (ready, true, Some(Triage), Ending::Failed, false),
            (ready, false, None, Ending::Waiting, true),
            (ready, false, Some(Triage), Ending::Waiting, false),
            (ready, true, None, Ending::Held, false),
            (
                ready,
                true,
                None,
                Ending::Reached(Goal::ReadyForReview),
                false,
            ),
            (ready, true, None, Ending::BeforeDelivery, false),
            (ready, true, None, Ending::NotReproduced, false),
        ] {
            assert_eq!(
                owes_comment(before, started, stop, ending),
                owed,
                "{before:?}, started {started}, stopped {}, {ending:?}",
                stop.is_some()
            );
        }
    }

    #[test]
    fn a_failed_label_edit_still_owes_the_recovery_comment() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]).failing(Fails::Edit);
        let claimed = claimed(&mut issue, &url);

        assert!(claimed.end(&mut issue, Ending::Failed));
    }

    #[test]
    fn a_comment_that_cannot_be_posted_is_a_warning_and_changes_no_label() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);
        claimed.end(&mut issue, Ending::Failed);
        issue = issue.failing(Fails::Comment);
        issue.calls.clear();

        claimed.post(&mut issue, "Recovery");

        assert_eq!(
            issue.calls,
            [
                step("commenting on #7 with how its run ended"),
                Call::Comment("Recovery".to_string()),
                Call::Interrupted,
                Call::Warn {
                    error: "gh: Comment failed: HTTP 502".to_string(),
                    warning: "could not comment on #7 with how its run ended; the run's outcome \
                              is unchanged"
                        .to_string(),
                },
            ]
        );
        assert_eq!(labels(&issue), ["agent-failed"]);
    }
}
