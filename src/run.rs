//! One Run: from an Issue URL to a checked PR, or to a Failed run.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

use crate::failed_run::{self, FailedRun};
use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::prompt;
use crate::session;
use crate::worktree::Worktree;

/// Take `issue_url` to a ready PR and return the PR's URL. Any failure after
/// the worktree exists goes through the Failed run path. The worktree, the
/// local Issue branch and the plugin directory are gone when this returns.
pub fn run(issue_url: &str) -> Result<String, FailedRun> {
    let issue = IssueUrl::parse(issue_url)?;
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    let origin = launch.run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        return Err(anyhow!(
            "origin mismatch: {issue_url} is not in the repository at origin {origin}"
        )
        .into());
    }
    let base = launch.run(&["symbolic-ref", "--short", "HEAD"])?;
    let branch = format!("issue-{}", issue.number);

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let worktree = Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?;
    let log = session::log_path(&issue, &timestamp, "implement")?;
    implement(&issue, &worktree, &base, &log)
        .map_err(|error| failed_run::fail(&issue, &worktree, &base, &log, error))
}

/// The implement session and the checks on the PR it opened.
fn implement(issue: &IssueUrl, worktree: &Worktree, base: &str, log: &Path) -> Result<String> {
    let branch = worktree.branch();
    let plugin = Plugin::write()?;
    session::run(
        worktree.path(),
        plugin.path(),
        &prompt::fresh(issue, base, branch),
        log,
    )?;
    worktree.git().run(&["push", "origin", branch])?;

    let pr = github::pull_request_for(issue, branch)?.context("no PR found")?;
    if !pr.is_open() {
        bail!("PR {} is {}, not open", pr.url, pr.state.to_lowercase());
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    if pr.is_draft {
        github::mark_ready(issue, branch)?;
    }
    if interrupt::requested() {
        bail!("interrupted");
    }
    Ok(pr.url)
}
