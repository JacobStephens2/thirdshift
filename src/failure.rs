//! The Failed run path's git side: the failure commit and its push.

use anyhow::Result;
use chrono::{SecondsFormat, Utc};

use crate::worktree::Worktree;

/// Commit everything in `worktree`, uncommitted work included, as the failure
/// commit for `reason`, and push the Issue branch. An unfinished merge is
/// aborted first. Does neither if the branch has no changes against `base`,
/// so no empty Issue branch appears on origin.
pub fn commit_and_push(worktree: &Worktree, base: &str, reason: &str) -> Result<()> {
    let git = worktree.git();
    if git.succeeds(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])? {
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
    git.run(&[
        "commit",
        "-q",
        "--allow-empty",
        "--no-verify",
        "-m",
        &message,
    ])?;
    git.run(&["push", "origin", worktree.branch()])?;
    Ok(())
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
