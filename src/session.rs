//! Headless Claude Code sessions and their logs.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::issue::IssueUrl;
use crate::progress::{self, Progress};

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
/// plugin at `plugin_dir` loaded, streaming its output to `log` and condensing
/// it to progress lines on stderr, each labelled `kind`.
pub fn run(kind: &str, worktree: &Path, plugin_dir: &Path, prompt: &str, log: &Path) -> Result<()> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let started = Instant::now();
    let mut child = Command::new("claude")
        .args(["-p", "--permission-mode", "auto", "--plugin-dir"])
        .arg(plugin_dir)
        .args(["--output-format", "stream-json", "--verbose"])
        .arg(prompt)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .context("could not run claude")?;

    let stream = child.stdout.take().context("no stdout from claude")?;
    let mut progress = Progress::default();
    let followed = follow(kind, stream, &mut log_file, log, &mut progress);
    if followed.is_err() {
        // Nothing reads its output any more, so it could block forever.
        let _ = child.kill();
    }
    let status = child.wait().context("could not wait for claude")?;

    let elapsed = minutes_and_seconds(started.elapsed());
    let summary = progress
        .summary()
        .map_or(String::new(), |summary| format!(": {summary}"));
    progress::step(format_args!(
        "{kind}: session ended after {elapsed}{summary}"
    ));
    followed?;
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

/// Copy every line of `stream` to `log_file` and print the progress lines it
/// condenses to, until the stream ends.
fn follow(
    kind: &str,
    stream: impl Read,
    log_file: &mut File,
    log: &Path,
    progress: &mut Progress,
) -> Result<()> {
    let mut stream = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        if stream
            .read_until(b'\n', &mut line)
            .context("could not read the session stream")?
            == 0
        {
            return Ok(());
        }
        log_file
            .write_all(&line)
            .with_context(|| format!("could not write {}", log.display()))?;
        for condensed in progress.condense(&String::from_utf8_lossy(&line)) {
            progress::step(format_args!("{kind}: {condensed}"));
        }
    }
}

/// `5m 32s`, or `8s` under a minute.
fn minutes_and_seconds(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds / 60 {
        0 => format!("{seconds}s"),
        minutes => format!("{minutes}m {}s", seconds % 60),
    }
}
