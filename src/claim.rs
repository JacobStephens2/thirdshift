//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on.

use anyhow::{Context, Result};

use crate::github;
use crate::issue::IssueUrl;
use crate::progress;
use crate::spec_run::READY_FOR_AGENT;

/// The label of a Claimed issue.
const IN_PROGRESS: &str = "in-progress";

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
    labels.iter().any(|name| name.eq_ignore_ascii_case(label))
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
