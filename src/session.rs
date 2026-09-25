//! Headless Claude Code sessions and their logs.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::interrupt;
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

/// How a session that exited cleanly ended.
pub struct Ended {
    /// The session's id, to resume it by.
    pub session_id: Option<String>,
    /// Descriptions of the background tasks killed as the session ended: work
    /// it was still waiting on.
    pub killed_background_work: Vec<String>,
}

/// Run `claude` headless in auto mode in `worktree`, with the Factory skills
/// plugin at `plugin_dir` loaded, streaming its output to `log` and condensing
/// it to progress lines on stderr, each labelled `kind`. With `resume`, the
/// session with that id continues, given `prompt`. An interrupt stops the
/// session and fails with `interrupted`.
pub fn run(
    kind: &str,
    worktree: &Path,
    plugin_dir: &Path,
    resume: Option<&str>,
    prompt: &str,
    log: &Path,
) -> Result<Ended> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let started = Instant::now();
    let mut command = Command::new("claude");
    command
        .args(["-p", "--permission-mode", "auto", "--plugin-dir"])
        .arg(plugin_dir)
        .args(["--output-format", "stream-json", "--verbose"]);
    if let Some(session_id) = resume {
        command.args(["--resume", session_id]);
    }
    let mut child = command
        .arg(prompt)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Its own process group, so thirdshift decides how it is stopped and
        // can stop everything it started.
        .process_group(0)
        .spawn()
        .context("could not run claude")?;

    // Follow the stream on its own thread, so this one can watch for an
    // interrupt while the session runs.
    let stream = child.stdout.take().context("no stdout from claude")?;
    let group = -(child.id() as libc::pid_t);
    let (kind_owned, log_owned) = (kind.to_string(), log.to_path_buf());
    let follower = thread::spawn(move || {
        let mut progress = Progress::default();
        let followed = follow(
            &kind_owned,
            stream,
            &mut log_file,
            &log_owned,
            &mut progress,
        );
        if followed.is_err() {
            // Nothing reads its output any more, so it could block forever.
            // SAFETY: kill has no memory-safety preconditions.
            unsafe { libc::kill(group, libc::SIGKILL) };
        }
        (followed, progress)
    });
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
    let (followed, progress) = follower
        .join()
        .map_err(|_| anyhow!("the session stream reader panicked"))?;

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
    Ok(Ended {
        session_id: progress.session_id().map(String::from),
        killed_background_work: progress
            .killed_background_work()
            .into_iter()
            .map(String::from)
            .collect(),
    })
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
