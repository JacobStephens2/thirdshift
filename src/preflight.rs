//! Pre-flight checks: everything that must hold before a Run creates
//! anything.

use anyhow::{Result, bail};

use crate::git::Git;
use crate::github;
use crate::issue::IssueUrl;

/// Check that a Run on `issue` from the `launch` repository makes sense. The
/// Base branch is checked separately, by [`check_base_branch`], once Issue
/// branch selection has picked it.
pub fn check(launch: &Git, issue: &IssueUrl) -> Result<()> {
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
    Ok(())
}

/// Check that the Base branch `base` exists on origin and that the local
/// `base`, if any, has no commits origin lacks.
pub fn check_base_branch(launch: &Git, base: &str) -> Result<()> {
    let on_origin = launch.run(&[
        "ls-remote",
        "--heads",
        "origin",
        &format!("refs/heads/{base}"),
    ])?;
    if on_origin.is_empty() {
        bail!("base branch {base} does not exist on origin; push it first");
    }
    launch.run(&["fetch", "origin", base])?;
    let local = format!("refs/heads/{base}");
    if launch
        .run(&["rev-parse", "--verify", "--quiet", &local])
        .is_err()
    {
        return Ok(());
    }
    let ahead = launch.run(&["rev-list", "--count", &format!("origin/{base}..{base}")])?;
    if ahead != "0" {
        bail!("local {base} is {ahead} commit(s) ahead of origin/{base}; push them first");
    }
    Ok(())
}
