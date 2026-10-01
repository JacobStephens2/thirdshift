//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on. It is released when that run ends with
//! nothing on origin to take over, and removed once its Self-merge has left
//! the issue closed.

use anyhow::{Context, Result};

use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::spec_run::READY_FOR_AGENT;

/// The label of a Claimed issue.
pub const IN_PROGRESS: &str = "in-progress";

/// The description the `in-progress` label is added to the repository with
/// if the repository lacks it.
const IN_PROGRESS_DESCRIPTION: &str = "A Claim: thirdshift has taken this issue";

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
pub fn is_on(labels: &[String]) -> bool {
    github::has_label(labels, IN_PROGRESS)
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
    let mut labels = github::issue_labels(issue)?;
    let (claimed, ready) = (is_on(&labels), github::has_label(&labels, READY_FOR_AGENT));
    let claim = Claim {
        issue,
        added_in_progress: !claimed,
        removed_ready_for_agent: ready,
    };
    if claimed && !ready {
        return Ok(claim);
    }
    if ready {
        progress::step(format_args!(
            "labelling #{} {IN_PROGRESS}, in place of {READY_FOR_AGENT}",
            issue.number
        ));
    } else {
        progress::step(format_args!("labelling #{} {IN_PROGRESS}", issue.number));
    }
    if !claimed {
        github::ensure_labels(
            &issue.repo_slug(),
            &[(IN_PROGRESS, IN_PROGRESS_DESCRIPTION)],
        )?;
    }
    labels.retain(|name| !name.eq_ignore_ascii_case(READY_FOR_AGENT));
    github::set_labels_adding(issue, &labels, &[IN_PROGRESS])?;
    Ok(claim)
}

impl Claim<'_> {
    /// Release the Claim if the Run or the Spec run that made it ended with
    /// nothing on origin to take over: neither `branch`, its Issue branch or
    /// Spec branch, which `launch` asks origin for, nor a pull request from
    /// it. The issue's labels then go back as they were before the Claim:
    /// `in-progress` comes off if the Claim added it, and `ready-for-agent`
    /// goes back if the Claim took it off, in one request that keeps the
    /// issue's other labels, those added since included. An issue that is no
    /// longer `in-progress` is left as it is: someone took the Claim off
    /// meanwhile. A Claim that changed no label has nothing to put back, and
    /// makes no request.
    ///
    /// The run has ended as it has, so this never fails: a failure is a
    /// warning naming what to run by hand, and an interrupt doesn't stop it.
    pub fn release_if_nothing_on_origin(&self, launch: &Git, branch: &str) {
        if !self.added_in_progress && !self.removed_ready_for_agent {
            return;
        }
        if let Err(error) = interrupt::retry_if_interrupted(|| {
            self.put_labels_back_unless_on_origin(launch, branch)
        }) {
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
    fn put_labels_back_unless_on_origin(&self, launch: &Git, branch: &str) -> Result<()> {
        let issue = self.issue;
        if launch.on_origin(branch)? || github::pull_request_for(issue, branch)?.is_some() {
            return Ok(());
        }
        let mut labels = github::issue_labels(issue)?;
        if !is_on(&labels) {
            return Ok(());
        }
        let number = issue.number;
        if !self.removed_ready_for_agent {
            progress::step(format_args!(
                "releasing the Claim on #{number}: removing {IN_PROGRESS}"
            ));
            return github::remove_label(issue, IN_PROGRESS);
        }
        if self.added_in_progress {
            progress::step(format_args!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT}, \
                 in place of {IN_PROGRESS}"
            ));
            labels.retain(|name| !name.eq_ignore_ascii_case(IN_PROGRESS));
        } else {
            progress::step(format_args!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT} again"
            ));
        }
        github::set_labels_adding(issue, &labels, &[READY_FOR_AGENT])
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
                    github::remove_label_command(self.issue, IN_PROGRESS)
                ),
            );
        }
    }

    /// [`Claim::remove_if_closed`], its failure as `gh` gave it.
    fn unlabel_if_closed(&self) -> Result<()> {
        let issue = github::issue(self.issue)?;
        if issue.is_open || !is_on(&issue.labels) {
            return Ok(());
        }
        progress::step(format_args!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        github::remove_label(self.issue, IN_PROGRESS)
    }

    /// The commands that release the Claim by hand.
    fn release_by_hand(&self) -> String {
        let mut commands = Vec::new();
        if self.added_in_progress {
            commands.push(github::remove_label_command(self.issue, IN_PROGRESS));
        }
        if self.removed_ready_for_agent {
            commands.push(github::add_label_command(self.issue, READY_FOR_AGENT));
        }
        commands.join(" && ")
    }
}
