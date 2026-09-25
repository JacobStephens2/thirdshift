//! Headless Claude Code sessions and their logs.

use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::interrupt;
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
/// plugin at `plugin_dir` loaded, streaming its output to `log`. An interrupt
/// stops the session and fails with `interrupted`.
pub fn run(worktree: &Path, plugin_dir: &Path, prompt: &str, log: &Path) -> Result<()> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let mut child = Command::new("claude")
        .args(["-p", "--permission-mode", "auto", "--plugin-dir"])
        .arg(plugin_dir)
        .args(["--output-format", "stream-json", "--verbose"])
        .arg(prompt)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(log_file)
        // Its own process group, so thirdshift decides how it is stopped and
        // can stop everything it started.
        .process_group(0)
        .spawn()
        .context("could not run claude")?;
    let status = loop {
        if interrupt::requested() {
            stop(&mut child);
            bail!("interrupted");
        }
        if let Some(status) = child.try_wait().context("could not wait for claude")? {
            break status;
        }
        thread::sleep(POLL);
    };
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

const POLL: Duration = Duration::from_millis(100);

/// How long a session gets to exit after SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// Stop `child`'s process group: SIGTERM, then SIGKILL if it outlives
/// `STOP_GRACE`.
fn stop(child: &mut Child) {
    let group = -(child.id() as libc::pid_t);
    // SAFETY: kill has no memory-safety preconditions.
    unsafe { libc::kill(group, libc::SIGTERM) };
    let deadline = Instant::now() + STOP_GRACE;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(POLL);
    }
    // SAFETY: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
    let _ = child.wait();
}
