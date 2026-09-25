//! One Run: from an Issue URL to a checked PR, or to a Failed run.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::failure;
use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::prompt;
use crate::session;
use crate::worktree::Worktree;

/// Why a Run did not end with a ready PR, and what the user should see.
pub struct Failure {
    pub error: anyhow::Error,
    /// The Issue branch's open PR, if it has one.
    pub pr_url: Option<String>,
    /// The most recent session log, if a session was started.
    pub log: Option<PathBuf>,
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Failure {
            error,
            pr_url: None,
            log: None,
        }
    }
}

/// Take `issue_url` to a ready PR and return the PR's URL. Any failure after
/// the worktree exists goes through the Failed run path. The worktree, the
/// local Issue branch and the plugin directory are gone when this returns.
pub fn run(issue_url: &str) -> Result<String, Failure> {
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
        .map_err(|error| fail(&issue, &worktree, &base, &log, error))
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
    if pr.state != "OPEN" {
        bail!("PR {} is {}, not open", pr.url, pr.state.to_lowercase());
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    if pr.is_draft {
        github::set_draft(issue, branch, false)?;
    }
    Ok(pr.url)
}

/// The Failed run path: push the work, send an open PR back to draft, and
/// describe the failure. Problems along the way are reported, not raised, so
/// the original `error` is what the Run fails with.
fn fail(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    log: &Path,
    error: anyhow::Error,
) -> Failure {
    // An interrupt can surface as some other error, such as a killed git.
    let error = if interrupt::requested() {
        anyhow!("interrupted")
    } else {
        error
    };
    let reason = error.to_string();
    if let Err(problem) = failure::commit_and_push(worktree, base, &reason) {
        eprintln!("thirdshift: could not push the failed run's work: {problem:#}");
    }
    let pr_url = match open_pr_as_draft(issue, worktree.branch()) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            eprintln!("thirdshift: could not convert the PR to a draft: {problem:#}");
            None
        }
    };
    Failure {
        error,
        pr_url,
        log: log.exists().then(|| log.to_path_buf()),
    }
}

/// Convert the open PR for `branch`, if there is one, to a draft, and return
/// its URL.
fn open_pr_as_draft(issue: &IssueUrl, branch: &str) -> Result<Option<String>> {
    let Some(pr) = github::pull_request_for(issue, branch)?.filter(|pr| pr.state == "OPEN") else {
        return Ok(None);
    };
    if !pr.is_draft {
        github::set_draft(issue, branch, true)?;
    }
    Ok(Some(pr.url))
}
