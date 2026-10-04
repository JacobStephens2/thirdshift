//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on. It is released when that run ends with
//! nothing on origin to take over, and removed once its Self-merge has left
//! the issue closed.

use anyhow::{Context, Result};

use crate::branch;
use crate::git::Git;
use crate::github::{self, ListedIssue};
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{self, Edit, Label, Labels, READY_FOR_AGENT};
use crate::progress;

/// The label of a Claimed issue.
pub const IN_PROGRESS: Label =
    Label::new("in-progress", "A Claim: thirdshift has taken this issue");

/// The Claim a Run or a Spec run made on its issue, with what making it
/// changed, which is what releasing it puts back.
pub struct Claim<'a> {
    issue: &'a IssueUrl,
    /// Whether making it added `in-progress`: the issue was not Claimed
    /// already.
    added_in_progress: bool,
    /// Whether making it took `ready-for-agent` off the issue.
    removed_ready_for_agent: bool,
}

/// Whether an issue with `labels` carries a Claim, whatever else it is
/// labelled.
pub fn is_on(labels: &Labels) -> bool {
    labels.has(IN_PROGRESS)
}

/// Make the Claim on `issue`: label it `in-progress`, in place of
/// `ready-for-agent` if it has that, in one request that keeps its other
/// labels, having added `in-progress` to the repository if it lacks it. An
/// issue already Claimed, `in-progress` and not `ready-for-agent`, is left as
/// it is, with no request made. A failure names the Claim as what could not
/// be made.
pub fn make(issue: &IssueUrl) -> Result<Claim<'_>> {
    label_in_progress(issue)
        .with_context(|| format!("could not make the Claim on #{}", issue.number))
}

/// [`make`], its failure as `gh` gave it.
fn label_in_progress(issue: &IssueUrl) -> Result<Claim<'_>> {
    let edit = Edit::read(issue, &[READY_FOR_AGENT], &[IN_PROGRESS])?;
    let claim = Claim {
        issue,
        added_in_progress: edit.puts_on(IN_PROGRESS),
        removed_ready_for_agent: edit.takes_off(READY_FOR_AGENT),
    };
    if !claim.added_in_progress && !claim.removed_ready_for_agent {
        return Ok(claim);
    }
    if claim.removed_ready_for_agent {
        progress::step(format_args!(
            "labelling #{} {IN_PROGRESS}, in place of {READY_FOR_AGENT}",
            issue.number
        ));
    } else {
        progress::step(format_args!("labelling #{} {IN_PROGRESS}", issue.number));
    }
    edit.apply()?;
    Ok(claim)
}

impl Claim<'_> {
    /// Release the Claim if the Run or the Spec run that made it ended with
    /// nothing on origin to take over: no Issue branch for its issue, which
    /// for a Spec is its Spec branch, as `launch` asks origin, and no pull
    /// request from one, open, merged or closed. So an issue is released
    /// only if it was never started, as a Ready issue never was. The issue's
    /// labels then go back as they were before the Claim:
    /// `in-progress` comes off if the Claim added it, and `ready-for-agent`
    /// goes back if the Claim took it off, in one request that keeps the
    /// issue's other labels, those added since included. An issue that is no
    /// longer `in-progress` is left as it is: someone took the Claim off
    /// meanwhile. A Claim that changed no label has nothing to put back, and
    /// makes no request.
    ///
    /// The run has ended as it has, so this never fails: a failure is a
    /// warning naming what to run by hand, and an interrupt doesn't stop it.
    pub fn release_if_nothing_on_origin(&self, launch: &Git) {
        if !self.added_in_progress && !self.removed_ready_for_agent {
            return;
        }
        if let Err(error) =
            interrupt::retry_if_interrupted(|| self.put_labels_back_unless_on_origin(launch))
        {
            progress::warn(
                &error,
                format_args!(
                    "could not release the Claim on #{}, so if nothing of the run is on origin, \
                     release it by hand: {}",
                    self.issue.number,
                    self.release_by_hand()
                ),
            );
        }
    }

    /// [`Claim::release_if_nothing_on_origin`], its failure as `git` or `gh`
    /// gave it.
    fn put_labels_back_unless_on_origin(&self, launch: &Git) -> Result<()> {
        let issue = self.issue;
        if branch::started(launch, issue)?.is_some() {
            return Ok(());
        }
        let (off, on) = self.put_back();
        let edit = Edit::read(issue, off, on)?;
        if !is_on(edit.labels()) {
            return Ok(());
        }
        let number = issue.number;
        if !self.removed_ready_for_agent {
            progress::step(format_args!(
                "releasing the Claim on #{number}: removing {IN_PROGRESS}"
            ));
        } else if self.added_in_progress {
            progress::step(format_args!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT}, \
                 in place of {IN_PROGRESS}"
            ));
        } else {
            progress::step(format_args!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT} again"
            ));
        }
        edit.apply()
    }

    /// The labels releasing the Claim takes off and puts on: `in-progress`
    /// off if making it added that, and `ready-for-agent` on if making it
    /// took that off.
    fn put_back(&self) -> (&'static [Label], &'static [Label]) {
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

    /// Remove the Claim once the Self-merge of the Run or the Spec run that
    /// made it has left its issue closed: `in-progress` comes off, in one
    /// request that keeps the issue's other labels. An issue still open, as
    /// when closing it failed, keeps it, and one no longer `in-progress` is
    /// left as it is.
    ///
    /// Like the other steps after a merge, this never fails: a failure is a
    /// warning naming the command to run by hand, and an interrupt doesn't
    /// stop it.
    pub fn remove_if_closed(&self) {
        if let Err(error) = interrupt::retry_if_interrupted(|| self.unlabel_if_closed()) {
            progress::warn(
                &error,
                format_args!(
                    "could not remove {IN_PROGRESS} from issue #{}, so remove it by hand: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &[IN_PROGRESS], &[])
                ),
            );
        }
    }

    /// [`Claim::remove_if_closed`], its failure as `gh` gave it.
    fn unlabel_if_closed(&self) -> Result<()> {
        let issue = github::issue(self.issue)?;
        if issue.is_open {
            return Ok(());
        }
        let edit = Edit::of(self.issue, issue.labels, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(());
        }
        progress::step(format_args!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        edit.apply()
    }

    /// The commands that release the Claim by hand.
    fn release_by_hand(&self) -> String {
        let (off, on) = self.put_back();
        labels::by_hand(self.issue, off, on)
    }
}

/// How many open issues in `repo` carry a Claim, whoever started the Run or
/// Spec run that made it: what a Pickup run holds against the Claim limit.
pub fn open_count(repo: &Repo) -> Result<usize> {
    Ok(github::open_issues_labelled(&repo.slug(), IN_PROGRESS)?.len())
}

/// The Sweep: take `in-progress` off every closed issue in `repo` that still
/// carries it, as an issue merged by hand does, leaving its other labels. A
/// failure, to list them or to take the label off one, is only a warning.
pub fn sweep(repo: &Repo) {
    let closed = match github::closed_issues_labelled(&repo.slug(), IN_PROGRESS) {
        Ok(closed) => closed,
        Err(error) => {
            progress::step(format_args!(
                "warning: could not list the closed issues labelled {IN_PROGRESS}: {error:#}"
            ));
            return;
        }
    };
    for ListedIssue { issue, labels, .. } in closed {
        let edit = Edit::of(&issue, labels, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            continue;
        }
        progress::step(format_args!(
            "taking {IN_PROGRESS} off #{}, which is closed",
            issue.number
        ));
        if let Err(error) = edit.apply() {
            progress::step(format_args!(
                "warning: could not take {IN_PROGRESS} off #{}: {error:#}",
                issue.number
            ));
        }
    }
}
