//! Pre-flight checks: everything that must hold before a Run creates
//! anything.

use anyhow::{Result, bail};

use crate::git::Git;
use crate::github;
use crate::issue::IssueUrl;

/// Check that a Run on `issue` from the `launch` repository makes sense, and
/// return its Base branch.
pub fn check(launch: &Git, issue: &IssueUrl) -> Result<String> {
    let origin = launch.run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        bail!(
            "origin mismatch: {} is not in the repository at origin {origin}",
            issue.url
        );
    }
    if !github::issue_is_open(issue)? {
        bail!("issue #{} is closed", issue.number);
    }
    for key in ["user.name", "user.email"] {
        if launch.run(&["config", "--default", "", key])?.is_empty() {
            bail!("git {key} is not set; the agent needs it to commit");
        }
    }
    base_branch(launch)
}

/// The checked-out branch, provided it exists on origin and has no commits
/// origin lacks.
fn base_branch(launch: &Git) -> Result<String> {
    let base = launch.run(&["branch", "--show-current"])?;
    if base.is_empty() {
        bail!("HEAD is detached; check out the branch the work should be based on");
    }
    let on_origin = launch.run(&[
        "ls-remote",
        "--heads",
        "origin",
        &format!("refs/heads/{base}"),
    ])?;
    if on_origin.is_empty() {
        bail!("base branch {base} does not exist on origin; push it first");
    }
    launch.run(&["fetch", "origin", &base])?;
    let ahead = launch.run(&["rev-list", "--count", &format!("origin/{base}..{base}")])?;
    if ahead != "0" {
        bail!("local {base} is {ahead} commit(s) ahead of origin/{base}; push them first");
    }
    Ok(base)
}
