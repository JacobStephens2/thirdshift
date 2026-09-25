//! One Run: from an Issue URL to a checked PR.

use anyhow::{Context, Result, bail};

use crate::branch::{self, Selection};
use crate::git::Git;
use crate::github;
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
    let checked_out = launch.run(&["symbolic-ref", "--short", "HEAD"])?;
    let selection = branch::select(&launch, &issue)?;
    let branch = selection.branch().to_string();
    let base = match &selection {
        Selection::Continuation { pr: Some(pr), .. } => {
            if pr.base != checked_out {
                eprintln!(
                    "thirdshift: continuing {} and its PR {}, so the Base branch is {}, not the checked-out {checked_out}",
                    branch, pr.url, pr.base
                );
            }
            pr.base.clone()
        }
        _ => checked_out,
    };
    check_local_issue_branch(&launch, &branch, selection.origin_sha())?;

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
    if pr.state != "OPEN" {
        bail!("PR {} is {}, not open", pr.url, pr.state.to_lowercase());
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    Ok(pr.url)
}

/// A local Issue branch in the launch repository must match its origin copy
/// (`origin_sha`, or no origin copy at all): the run replaces it and deletes it
/// at cleanup, so anything else would destroy local-only commits.
fn check_local_issue_branch(launch: &Git, branch: &str, origin_sha: Option<&str>) -> Result<()> {
    let Ok(local_sha) = launch.run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ]) else {
        return Ok(());
    };
    match origin_sha {
        Some(origin_sha) if origin_sha == local_sha => Ok(()),
        Some(_) => bail!(
            "the local branch {branch} differs from origin/{branch}; push, reset or delete it first"
        ),
        None => {
            bail!("the local branch {branch} is not on origin; push, rename or delete it first")
        }
    }
}
