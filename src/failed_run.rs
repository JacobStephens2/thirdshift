//! The Failed run path: what happens when a Run can't end with a ready PR.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use chrono::{SecondsFormat, Utc};

use crate::github;
use crate::host;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::worktree::Worktree;

/// What starts the line naming a Failed run's session log on stderr, which
/// a Spec run reads back from a Ticket's Run.
pub const SESSION_LOG: &str = "session log: ";

/// Why a Run did not end with a ready PR, and what the user should see.
pub struct FailedRun {
    pub error: anyhow::Error,
    /// The Issue branch's open PR, if it has one.
    pub pr_url: Option<String>,
    /// The most recent session log, if a session was started.
    pub log: Option<PathBuf>,
    /// Whether the Run was interrupted, as it was when the Run failed.
    pub interrupted: bool,
    /// In a Spec run, a line on each Ticket it landed or did not get done;
    /// empty in a Run.
    pub ticket_lines: Vec<String>,
    /// What became of the Base fix it started, if it started one.
    pub base_fix: Option<String>,
}

/// A merge GitHub refused when a round of the Repair loop found nothing to
/// fix. The Run fails, but, as it can do no more, leaves the PR ready for review
/// rather than a draft. It is the context of the merge error.
#[derive(Debug)]
pub struct PolicyRefusal;

impl fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the merge was refused with nothing left to fix, so the PR stays ready for review",
        )
    }
}

/// A failure with nothing to push or clean up, as before the worktree exists.
impl From<anyhow::Error> for FailedRun {
    fn from(error: anyhow::Error) -> Self {
        let interrupted = interrupt::requested();
        FailedRun {
            error: interrupted_or(error, interrupted),
            pr_url: None,
            log: None,
            interrupted,
            ticket_lines: Vec::new(),
            base_fix: None,
        }
    }
}

/// `interrupted` if the Run was, else `error`: an interrupt can surface as
/// some other error, such as a killed git.
fn interrupted_or(error: anyhow::Error, interrupted: bool) -> anyhow::Error {
    if interrupted {
        anyhow!("interrupted")
    } else {
        error
    }
}

/// Take the Run in `worktree` down the Failed run path: commit and push the
/// work, and send an open PR back to draft. A `PolicyRefusal` does neither,
/// so the PR stays ready on the head whose CI was watched. Problems along the
/// way are reported, not raised, so `error` is what the Run fails with. The
/// worktree is cleaned up, or kept if its work may not have reached origin.
pub fn fail(
    issue: &IssueUrl,
    worktree: Worktree,
    base: &str,
    log: &Path,
    error: anyhow::Error,
) -> FailedRun {
    let interrupted = interrupt::requested();
    let error = interrupted_or(error, interrupted);
    // Only the first line: the reason goes in the failure commit's subject.
    let reason = error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    // Everything is already on origin: the Repair loop pushed the head.
    let keep_ready = error.is::<PolicyRefusal>();
    let pushed = if keep_ready {
        Ok(())
    } else {
        commit_and_push(&worktree, base, &reason)
    };
    if let Err(problem) = &pushed {
        progress::step(format_args!(
            "could not push the failed run's work, so it may exist only locally: {problem:#}"
        ));
    }
    let pr_url = match open_pr_url(issue, worktree.branch(), keep_ready) {
        Ok(pr_url) => pr_url,
        Err(problem) => {
            progress::step(format_args!(
                "could not convert the PR to a draft: {problem:#}"
            ));
            None
        }
    };
    // Last, since keeping the worktree lets go of it.
    if pushed.is_err() {
        worktree.keep();
    }
    FailedRun {
        error,
        pr_url,
        log: log.exists().then(|| log.to_path_buf()),
        interrupted,
        ticket_lines: Vec::new(),
        base_fix: None,
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
        host = host::name().as_deref().unwrap_or("unknown"),
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

/// The URL of the open PR for `branch`, if there is one, converted to a draft
/// unless `keep_ready`.
fn open_pr_url(issue: &IssueUrl, branch: &str, keep_ready: bool) -> Result<Option<String>> {
    let Some(pr) = github::pull_request_for(issue, branch)?.filter(|pr| pr.is_open()) else {
        return Ok(None);
    };
    if !pr.is_draft && !keep_ready {
        github::convert_to_draft(issue, branch)?;
    }
    Ok(Some(pr.url))
}
