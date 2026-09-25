//! One Run: from an Issue URL to a checked PR.

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::github;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::progress;
use crate::prompt;
use crate::session;
use crate::worktree::Worktree;

/// Take `issue_url` to a PR and return the PR's URL. The worktree, the local
/// Issue branch and the plugin directory are gone when this returns.
pub fn run(issue_url: &str) -> Result<String> {
    let issue = IssueUrl::parse(issue_url)?;
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    let origin = launch.run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        bail!("origin mismatch: {issue_url} is not in the repository at origin {origin}");
    }
    let base = launch.run(&["symbolic-ref", "--short", "HEAD"])?;
    let branch = format!("issue-{}", issue.number);

    let worktree = Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?;
    let plugin = Plugin::write()?;
    let log = session::log_path(&issue, &timestamp, "implement")?;
    progress::step(format_args!("logging the session to {}", log.display()));
    session::run(
        "implement",
        worktree.path(),
        plugin.path(),
        &prompt::fresh(&issue, &base, &branch),
        &log,
    )?;
    progress::step(format_args!("pushing {branch}"));
    worktree.git().run(&["push", "origin", &branch])?;

    progress::step("checking the PR");
    let pr = github::pull_request_for(&issue, &branch)?.context("no PR found")?;
    if pr.state != "OPEN" {
        bail!("PR {} is {}, not open", pr.url, pr.state.to_lowercase());
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    Ok(pr.url)
}
