//! The Failed run path: what happens when a Run can't end with a ready PR.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::{SecondsFormat, Utc};

use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::worktree::Worktree;

/// Why a Run did not end with a ready PR, and what the user should see.
pub struct FailedRun {
    pub error: anyhow::Error,
    /// The Issue branch's open PR, if it has one.
    pub pr_url: Option<String>,
    /// The most recent session log, if a session was started.
    pub log: Option<PathBuf>,
}

/// A failure before the worktree exists: nothing to push or clean up.
impl From<anyhow::Error> for FailedRun {
    fn from(error: anyhow::Error) -> Self {
        FailedRun {
            error,
            pr_url: None,
            log: None,
        }
    }
}

/// Take the Run in `worktree` down the Failed run path: commit and push the
/// work, and send an open PR back to draft. Problems along the way are
/// reported, not raised, so `error` is what the Run fails with. The worktree
/// is cleaned up, or kept if its work may not have reached origin.
pub fn fail(
    issue: &IssueUrl,
    worktree: Worktree,
    base: &str,
    log: &Path,
    error: anyhow::Error,
) -> FailedRun {
    // An interrupt can surface as some other error, such as a killed git.
    let error = if interrupt::requested() {
        anyhow!("interrupted")
    } else {
        error
    };
    // Only the first line: the reason goes in the failure commit's subject.
    let reason = error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let pushed = commit_and_push(&worktree, base, &reason);
    if let Err(problem) = &pushed {
        progress::step(format_args!(
            "could not push the failed run's work: {problem:#}"
        ));
    }
    let pr_url = match open_pr_as_draft(issue, worktree.branch()) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            progress::step(format_args!(
                "could not convert the PR to a draft: {problem:#}"
            ));
            None
        }
    };
    if pushed.is_err() {
        worktree.keep();
    }
    FailedRun {
        error,
        pr_url,
        log: log.exists().then(|| log.to_path_buf()),
    }
}

/// Commit everything in `worktree`, uncommitted work included, as the failure
/// commit for `reason`, and push the Issue branch. An unfinished merge is
/// aborted first. Does neither if the branch has no changes against `base`,
/// so no empty Issue branch appears on origin.
fn commit_and_push(worktree: &Worktree, base: &str, reason: &str) -> Result<()> {
    let git = worktree.git();
    if worktree.merge_in_progress()? {
        git.run(&["merge", "--abort"])?;
    }
    git.run(&["add", "-A"])?;
    if git.succeeds(&["diff", "--cached", "--quiet", &format!("origin/{base}")])? {
        return Ok(());
    }
    let message = format!(
        "thirdshift: failed run ({reason})\n\n\
         {timestamp}, host {host}. Uncommitted work at the time of failure is included in this commit.",
        timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        host = hostname(),
    );
    // No hooks: a hook that rejects the commit would strand the work.
    git.run(&[
        "commit",
        "-q",
        "--allow-empty",
        "--no-verify",
        "-m",
        &message,
    ])?;
    worktree.push()
}

/// Convert the open PR for `branch`, if there is one, to a draft, and return
/// its URL.
fn open_pr_as_draft(issue: &IssueUrl, branch: &str) -> Result<Option<String>> {
    let Some(pr) = github::pull_request_for(issue, branch)?.filter(|pr| pr.is_open()) else {
        return Ok(None);
    };
    if !pr.is_draft {
        github::convert_to_draft(issue, branch)?;
    }
    Ok(Some(pr.url))
}

fn hostname() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: the pointer and length describe `buffer`, which outlives the call.
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return "unknown".to_string();
    }
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}
