//! Headless Claude Code sessions and their logs.

use std::cell::RefCell;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::logs;
use crate::plugin::Plugin;
use crate::progress::{self, Progress};
use crate::prompt;

/// Where a Run's or an Architect run's Session logs go: in `sessions/` under
/// the root of its repository's logs, under the User config's `logs.dir` or
/// `~/.thirdshift/logs`, each named for what is run and stamped with the
/// command's start stamp, which its Command log shares. Only once the logs
/// are [`logs::configured`].
pub struct Logs {
    /// What every log's name starts with: the issue's number for a Run,
    /// `architect` for an Architect run.
    name: String,
    dir: PathBuf,
}

impl Logs {
    /// The logs of the Run on `issue`.
    pub fn of_run(issue: &IssueUrl) -> Self {
        Logs {
            name: issue.number.to_string(),
            dir: logs::root(&issue.repo()).join("sessions"),
        }
    }

    /// The logs of the Architect run on `repo`.
    pub fn of_architect_run(repo: &Repo) -> Self {
        Logs {
            name: "architect".to_string(),
            dir: logs::root(repo).join("sessions"),
        }
    }

    /// Where a session's stream is logged:
    /// `<root>/sessions/<name>-<stamp>-<kind>.jsonl`.
    pub fn path(&self, kind: &str) -> PathBuf {
        self.dir
            .join(format!("{}-{}-{kind}.jsonl", self.name, logs::stamp()))
    }
}

/// Where a Run's or an Architect run's sessions run: in `worktree`, with the
/// Factory skills plugin loaded, each logged in its `logs`. It lasts for the
/// steps given to [`Sessions::within`], and so does the plugin.
pub struct Sessions<'a> {
    logs: &'a Logs,
    worktree: &'a Path,
    plugin: Plugin,
    /// The log of the session started last, a Resume included.
    last_log: RefCell<Option<PathBuf>>,
    /// How each session ended whose last ending, its Resume's if it got one,
    /// left background work running, as in "implement session ended with a
    /// background task still running (cargo test), which was killed", oldest
    /// first.
    endings_with_killed_work: RefCell<Vec<String>>,
}

impl<'a> Sessions<'a> {
    /// Write the Factory skills plugin, then take `steps`, which run their
    /// sessions through the `Sessions` they are given. Returns what `steps`
    /// came to, with the most recent session's log, if a session created it.
    /// If a session's last ending, its Resume's if it got one, left
    /// background work to be killed and `steps` then fail, the failure names
    /// that work ahead of its own cause: the session may have stopped short
    /// of its job. The plugin directory is gone when this returns.
    pub fn within<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let plugin = match Plugin::write() {
            Ok(plugin) => plugin,
            Err(error) => return (Err(error), None),
        };
        let sessions = Sessions {
            logs,
            worktree,
            plugin,
            last_log: RefCell::default(),
            endings_with_killed_work: RefCell::default(),
        };
        let taken = steps(&sessions);
        let log = sessions.last_log.into_inner().filter(|log| log.exists());
        let endings = sessions.endings_with_killed_work.into_inner();
        if endings.is_empty() {
            return (taken, log);
        }
        let taken = taken.map_err(|error| {
            error.context(format!("{}, and a later step failed", endings.join("; ")))
        });
        (taken, log)
    }

    /// Run a session given `prompt`, logged and labelled as `kind`. A session that ends with
    /// background work still running, which is killed with it, gets one
    /// Resume, as `<kind>-resume`. If the Resume ends the same way, or the
    /// session has no id to resume, the work may have been abandoned rather
    /// than awaited: a progress line names it and this succeeds, leaving the
    /// steps that follow to decide the outcome.
    pub fn run(&self, kind: &str, prompt: &str) -> Result<()> {
        self.run_to_final_message(kind, prompt).map(drop)
    }

    /// Run a session as [`Sessions::run`] does, and return its final message:
    /// what the agent said as it ended its last turn, its Resume's if it got
    /// one, if anything.
    pub fn run_to_final_message(&self, kind: &str, prompt: &str) -> Result<Option<String>> {
        let mut ended = self.start(kind, None, prompt)?;
        // What ended last, as the progress line on its killed work calls it.
        let mut ended_last = "session";
        let killed = ended.killed_background_work();
        if let (false, Some(session_id)) = (killed.is_empty(), ended.session_id()) {
            let (session_id, resume_prompt) = (session_id.to_string(), prompt::resume(&killed));
            progress::step(format_args!(
                "{kind}: background work was killed as the session ended; resuming it once"
            ));
            ended = self.start(&format!("{kind}-resume"), Some(&session_id), &resume_prompt)?;
            ended_last = "Resume";
        }
        let killed = ended.killed_background_work();
        if !killed.is_empty() {
            let ending = ending_with(&killed);
            progress::step(format_args!(
                "{kind}: the {ended_last} {ending}; carrying on, as it may have been abandoned"
            ));
            self.endings_with_killed_work
                .borrow_mut()
                .push(format!("{kind} session {ending}"));
        }
        Ok(ended.final_message().map(String::from))
    }

    /// Run one session as `kind`, logged under its own path, which becomes
    /// the last log.
    fn start(&self, kind: &str, resume: Option<&str>, prompt: &str) -> Result<Progress> {
        let log = self.logs.path(kind);
        progress::step(format_args!("logging the session to {}", log.display()));
        *self.last_log.borrow_mut() = Some(log.clone());
        run(
            kind,
            self.worktree,
            self.plugin.path(),
            resume,
            prompt,
            &log,
        )
    }
}

/// How a session ended whose `killed` background work, by description, was
/// still running: what follows "implement session" or "the Resume".
fn ending_with(killed: &[&str]) -> String {
    match killed {
        [task] => format!("ended with a background task still running ({task}), which was killed"),
        _ => format!(
            "ended with background tasks still running ({}), which were killed",
            killed.join("; ")
        ),
    }
}

/// Run `claude` headless in auto mode in `worktree`, with the Factory skills
/// plugin at `plugin_dir` loaded, streaming its output to `log` and condensing
/// it to progress lines on stderr, each labelled `kind`. With `resume`, the
/// session with that id continues, given `prompt`. Returns what the stream
/// showed once `claude` has exited cleanly. An interrupt stops the session and
/// fails with `interrupted`.
fn run(
    kind: &str,
    worktree: &Path,
    plugin_dir: &Path,
    resume: Option<&str>,
    prompt: &str,
    log: &Path,
) -> Result<Progress> {
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let started = Instant::now();
    let mut child = Command::new("claude")
        .args(claude_args(plugin_dir.as_os_str(), resume, prompt))
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
    Ok(progress)
}

/// The arguments every session runs `claude` with: headless in auto mode,
/// with the Factory skills plugin at `plugin_dir` loaded, streaming JSON. With
/// `resume`, the session with that id continues. The prompt comes last.
pub fn claude_args<'a>(
    plugin_dir: &'a OsStr,
    resume: Option<&'a str>,
    prompt: &'a str,
) -> Vec<&'a OsStr> {
    let mut args: Vec<&OsStr> = ["-p", "--permission-mode", "auto", "--plugin-dir"]
        .into_iter()
        .map(OsStr::new)
        .collect();
    args.push(plugin_dir);
    args.extend(["--output-format", "stream-json", "--verbose"].map(OsStr::new));
    if let Some(session_id) = resume {
        args.extend(["--resume", session_id].map(OsStr::new));
    }
    args.push(OsStr::new(prompt));
    args
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
