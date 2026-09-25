//! One Run: from an Issue URL to a checked PR.

use anyhow::{Context, Result, bail};

use crate::branch::{self, Selection};
use crate::git::Git;
use crate::github::{self, PrState};
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
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
    let checked_out = launch
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok();
    let selection = branch::select(&launch, &issue)?;
    let branch = selection.branch().to_string();
    let base = selection.base_branch(checked_out.as_deref())?;

    let (worktree, prompt) = match &selection {
        Selection::Fresh { .. } => (
            Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?,
            prompt::fresh(&issue, &base, &branch),
        ),
        Selection::Continuation { pr, .. } => (
            Worktree::continue_existing(&launch, &issue.repo, &branch, &base)?,
            prompt::continuation(
                &issue,
                &base,
                &branch,
                pr.as_ref().map(|pr| pr.url.as_str()),
            ),
        ),
    };
    let plugin = Plugin::write()?;
    let log = session::log_path(&issue, &timestamp, "implement")?;
    session::run(worktree.path(), plugin.path(), &prompt, &log)?;
    worktree.git().run(&["push", "origin", &branch])?;

    let pr = github::pull_request_for(&issue, &branch)?.context("no PR found")?;
    if pr.state != PrState::Open {
        bail!("PR {} is {}, not open", pr.url, pr.state);
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    Ok(pr.url)
}
