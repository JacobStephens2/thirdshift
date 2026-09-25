//! One Run: from an Issue URL to a checked PR, or to a Failed run.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

use crate::failed_run::{self, FailedRun};
use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::session;
use crate::worktree::{Merge, Worktree};

/// Take `issue` to a ready PR and return the PR's URL. Any failure after the
/// worktree exists goes through the Failed run path. The worktree, the local
/// Issue branch and the plugin directory are gone when this returns.
pub fn run(issue: &IssueUrl) -> Result<String, FailedRun> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    let base = preflight::check(&launch, issue)?;
    let branch = format!("issue-{}", issue.number);

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let worktree = Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?;
    let mut log = session::log_path(issue, &timestamp, "implement")?;
    implement(issue, &worktree, &base, &timestamp, &mut log)
        .map_err(|error| failed_run::fail(issue, &worktree, &base, &log, error))
}

/// The implement session, the checks on the PR it opened, and keeping that PR
/// mergeable. `log` is left at the most recent session's log.
fn implement(
    issue: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    timestamp: &str,
    log: &mut PathBuf,
) -> Result<String> {
    let branch = worktree.branch();
    let plugin = Plugin::write()?;
    // Every session runs in the worktree with the plugin loaded, logged as
    // `kind` under the Run's timestamp.
    let mut run_session = |kind: &str, prompt: &str| -> Result<()> {
        *log = session::log_path(issue, timestamp, kind)?;
        progress::step(format_args!("logging the session to {}", log.display()));
        session::run(kind, worktree.path(), plugin.path(), prompt, log)
    };

    run_session("implement", &prompt::fresh(issue, base, branch))?;
    worktree.push()?;

    progress::step("checking the PR");
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

    // Keep the PR mergeable: merge the Base branch, never rebase.
    if worktree.merge_base_branch(base)? == Merge::Conflicted {
        progress::step(format_args!(
            "merging origin/{base} conflicted; starting a Repair"
        ));
        run_session(
            "repair-1",
            &prompt::conflict_repair(issue, base, branch, &pr.url),
        )?;
        worktree.ensure_base_branch_merged(base)?;
    }
    worktree.push()?;
    if interrupt::requested() {
        bail!("interrupted");
    }
    Ok(pr.url)
}
