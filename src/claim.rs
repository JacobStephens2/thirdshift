//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on.

use anyhow::{Context, Result};

use crate::github::{self, ListedIssue};
use crate::issue::{IssueUrl, Repo};
use crate::progress;
use crate::spec_run::READY_FOR_AGENT;

/// The label of a Claimed issue.
pub const IN_PROGRESS: &str = "in-progress";

/// The description the `in-progress` label is added to the repository with
/// if the repository lacks it.
const IN_PROGRESS_DESCRIPTION: &str = "A Claim: thirdshift has taken this issue";

/// Whether an issue with `labels` carries a Claim, whatever else it is
/// labelled.
pub fn is_on(labels: &[String]) -> bool {
    has(labels, IN_PROGRESS)
}

/// Whether `label` is one of `labels`, whatever its case: GitHub's label
/// names are case-insensitive.
fn has(labels: &[String], label: &str) -> bool {
    spelling(labels, label).is_some()
}

/// `label` as `labels` spells it, whatever its case, if it is one of them.
fn spelling<'a>(labels: &'a [String], label: &str) -> Option<&'a String> {
    labels.iter().find(|name| name.eq_ignore_ascii_case(label))
}

/// Make the Claim on `issue`: label it `in-progress`, in place of
/// `ready-for-agent` if it has that, in one request that keeps its other
/// labels, having added `in-progress` to the repository if it lacks it. An
/// issue already Claimed, `in-progress` and not `ready-for-agent`, is left as
/// it is, with no request made. A failure names the Claim as what could not
/// be made.
pub fn make(issue: &IssueUrl) -> Result<()> {
    label_in_progress(issue)
        .with_context(|| format!("could not make the Claim on #{}", issue.number))
}

/// [`make`], its failure as `gh` gave it.
fn label_in_progress(issue: &IssueUrl) -> Result<()> {
    let mut labels = github::issue_labels(issue)?;
    let (claimed, ready) = (is_on(&labels), has(&labels, READY_FOR_AGENT));
    if claimed && !ready {
        return Ok(());
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
    github::set_labels_adding(issue, &labels, &[IN_PROGRESS])
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
        // As the issue spells it: GitHub's label names are case-insensitive.
        let Some(label) = spelling(&labels, IN_PROGRESS) else {
            continue;
        };
        progress::step(format_args!(
            "taking {IN_PROGRESS} off #{}, which is closed",
            issue.number
        ));
        if let Err(error) = github::remove_label(&issue, label) {
            progress::step(format_args!(
                "warning: could not take {IN_PROGRESS} off #{}: {error:#}",
                issue.number
            ));
        }
    }
}
