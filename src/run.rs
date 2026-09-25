//! One Run: from an Issue URL to a checked PR.

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::github;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::session;
use crate::worktree::{Merge, Worktree};

/// Take `issue` to a PR and return the PR's URL. The worktree, the local
/// Issue branch and the plugin directory are gone when this returns.
pub fn run(issue: &IssueUrl) -> Result<String> {
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    let base = preflight::check(&launch, issue)?;
    let branch = format!("issue-{}", issue.number);

    let worktree = Worktree::create_fresh(&launch, &issue.repo, &branch, &base)?;
    let plugin = Plugin::write()?;
    // Every session runs in the worktree with the plugin loaded, logged as
    // `kind` under the Run's timestamp.
    let run_session = |kind: &str, prompt: &str| -> Result<()> {
        let log = session::log_path(issue, &timestamp, kind)?;
        progress::step(format_args!("logging the session to {}", log.display()));
        session::run(kind, worktree.path(), plugin.path(), prompt, &log)
    };

    run_session("implement", &prompt::fresh(issue, &base, &branch))?;
    worktree.push()?;

    progress::step("checking the PR");
    let pr = github::pull_request_for(issue, &branch)?.context("no PR found")?;
    if pr.state != "OPEN" {
        bail!("PR {} is {}, not open", pr.url, pr.state.to_lowercase());
    }
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }

    // Keep the PR mergeable: merge the Base branch, never rebase.
    if worktree.merge_base_branch(&base)? == Merge::Conflicted {
        progress::step(format_args!(
            "merging origin/{base} conflicted; starting a Repair"
        ));
        run_session(
            "repair-1",
            &prompt::conflict_repair(issue, &base, &branch, &pr.url),
        )?;
        worktree.ensure_base_branch_merged(&base)?;
    }
    worktree.push()?;
    Ok(pr.url)
}
