//! Headless Claude Code sessions and their logs.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::issue::IssueUrl;

/// Where a session's stream is logged:
/// `~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-<kind>.jsonl`.
/// Every session in a Run shares the Run's `timestamp`.
pub fn log_path(issue: &IssueUrl, timestamp: &str, kind: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".thirdshift/logs").join(format!(
        "{}-{}-issue-{}-{timestamp}-{kind}.jsonl",
        issue.owner, issue.repo, issue.number
    )))
}

/// Run `claude` headless in auto mode in `worktree`, with the Factory skills
/// plugin at `plugin_dir` loaded, streaming its output to `log`.
pub fn run(worktree: &Path, plugin_dir: &Path, prompt: &str, log: &Path) -> Result<()> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let status = Command::new("claude")
        .args(["-p", "--permission-mode", "auto", "--plugin-dir"])
        .arg(plugin_dir)
        .args(["--output-format", "stream-json", "--verbose"])
        .arg(prompt)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(log_file)
        .status()
        .context("could not run claude")?;
    if !status.success() {
        bail!(
            "claude exited {}",
            status
                .code()
                .map_or("by signal".to_string(), |code| code.to_string())
        );
    }
    Ok(())
}
