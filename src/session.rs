//! Headless agent sessions, on Claude Code or Codex, and their logs.

use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::harness::{Choice, Harness};
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::logs;
use crate::progress::{self, Stream};
use crate::prompt;
use crate::skills;

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
/// Factory skills linked into it, on the Command's `harness`, each logged in
/// its `logs`. It lasts for the steps given to [`Sessions::within`].
pub struct Sessions<'a> {
    logs: &'a Logs,
    worktree: &'a Path,
    harness: &'a Choice,
    /// The log of the session started last, a Resume included.
    last_log: RefCell<Option<PathBuf>>,
    /// How each session ended whose last ending, its Resume's if it got one,
    /// left background work running, as in "implement session ended with a
    /// background task still running (cargo test), which was killed", oldest
    /// first.
    endings_with_killed_work: RefCell<Vec<String>>,
}

impl<'a> Sessions<'a> {
    /// Link the Factory skills into `worktree`, then take `steps`, which run
    /// their sessions on `harness` through the `Sessions` they are given.
    /// Returns what `steps` came to, with the most recent session's log, if a
    /// session created it. If a session's last ending, its Resume's if it got
    /// one, left background work to be killed and `steps` then fail, the
    /// failure names that work ahead of its own cause: the session may have
    /// stopped short of its job.
    pub fn within<T>(
        logs: &'a Logs,
        worktree: &'a Path,
        harness: &'a Choice,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        if let Err(error) = skills::link_into(worktree, harness.harness) {
            return (Err(error), None);
        }
        let sessions = Sessions {
            logs,
            worktree,
            harness,
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

    /// Run a session given `prompt`, logged and labelled as `kind`. A
    /// session that ends with background work still running, which is killed
    /// with it, gets one Resume, as `<kind>-resume`. If the Resume ends the
    /// same way, or the session has no id to resume, the work may have been
    /// abandoned rather than awaited: a progress line names it and this
    /// succeeds, leaving the steps that follow to decide the outcome.
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
    fn start(&self, kind: &str, resume: Option<&str>, prompt: &str) -> Result<Box<dyn Stream>> {
        let log = self.logs.path(kind);
        progress::step(format_args!("logging the session to {}", log.display()));
        *self.last_log.borrow_mut() = Some(log.clone());
        let harness = self.harness.harness;
        let args = session_args(self.harness, resume, prompt);
        let stream = progress::for_harness(harness, self.worktree);
        run(kind, harness, self.worktree, &args, &log, stream)
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

/// Run `harness`'s CLI with `args`, as [`session_args`] gives them, in
/// `worktree`, where it finds the Factory skills, streaming its output to
/// `log` and condensing it through `stream` to progress lines on stderr,
/// each labelled `kind`. Returns what the stream showed once the CLI has
/// exited cleanly, its turn not failed. Otherwise fails with the error the
/// stream gave, if any. An interrupt stops the session, as [`stop`] does,
/// and fails with `interrupted`.
fn run(
    kind: &str,
    harness: Harness,
    worktree: &Path,
    args: &[String],
    log: &Path,
    mut stream: Box<dyn Stream>,
) -> Result<Box<dyn Stream>> {
    let cli = harness.name();
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let started = Instant::now();
    let mut child = Command::new(cli)
        .args(args)
        .current_dir(worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Its own process group, so thirdshift decides how it is stopped and
        // can stop everything it started.
        .process_group(0)
        .spawn()
        .with_context(|| format!("could not run {cli}"))?;

    // Follow the stream on its own thread, so this one can watch for an
    // interrupt while the session runs.
    let output = child
        .stdout
        .take()
        .with_context(|| format!("no stdout from {cli}"))?;
    let group = -(child.id() as libc::pid_t);
    let (kind_owned, log_owned) = (kind.to_string(), log.to_path_buf());
    let follower = thread::spawn(move || {
        let followed = follow(
            &kind_owned,
            output,
            &mut log_file,
            &log_owned,
            stream.as_mut(),
        );
        if followed.is_err() {
            // Nothing reads its output any more, so it could block forever.
            // SAFETY: kill has no memory-safety preconditions.
            unsafe { libc::kill(group, libc::SIGKILL) };
        }
        (followed, stream)
    });
    let status = loop {
        if interrupt::requested() {
            stop(&mut child, harness);
            bail!("interrupted");
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("could not wait for {cli}"))?
        {
            break status;
        }
        thread::sleep(POLL);
    };
    let (followed, stream) = follower
        .join()
        .map_err(|_| anyhow!("the session stream reader panicked"))?;

    let elapsed = minutes_and_seconds(started.elapsed());
    let summary = stream
        .summary()
        .map_or(String::new(), |summary| format!(": {summary}"));
    progress::step(format_args!(
        "{kind}: session ended after {elapsed}{summary}"
    ));
    followed?;
    if status.success() && !stream.failed() {
        return Ok(stream);
    }
    let ended = if status.success() {
        "'s turn failed".to_string()
    } else {
        format!(
            " exited {}",
            status
                .code()
                .map_or("by signal".to_string(), |code| code.to_string())
        )
    };
    match stream.error() {
        Some(error) => bail!("{cli}{ended}: {error}"),
        None => bail!("{cli}{ended}"),
    }
}

/// The arguments a session runs `harness`'s CLI with, as [`claude_args`] or
/// [`codex_args`] give them, its prompt loading its skill with that
/// Harness's sigil.
fn session_args(harness: &Choice, resume: Option<&str>, prompt: &str) -> Vec<String> {
    match harness.harness {
        Harness::Claude => claude_args(harness, resume, prompt)
            .into_iter()
            .map(String::from)
            .collect(),
        Harness::Codex => codex_args(harness, resume, &codex_prompt(prompt)),
    }
}

/// `prompt` as Codex takes it: a first line that loads a Factory skill,
/// `/thirdshift-<skill>` as Claude's prompts write it, loads it as
/// `$thirdshift-<skill>`.
pub fn codex_prompt(prompt: &str) -> String {
    match prompt.strip_prefix("/thirdshift-") {
        Some(rest) => format!("$thirdshift-{rest}"),
        None => prompt.to_string(),
    }
}

/// The arguments every session runs `claude` with: headless in auto mode,
/// on the Model and Effort `harness` sets, if any, streaming JSON. With
/// `resume`, the session with that id continues. The prompt comes last.
pub fn claude_args<'a>(
    harness: &'a Choice,
    resume: Option<&'a str>,
    prompt: &'a str,
) -> Vec<&'a str> {
    let mut args = vec!["-p", "--permission-mode", "auto"];
    args.extend(harness.claude_args());
    args.extend(["--output-format", "stream-json", "--verbose"]);
    if let Some(session_id) = resume {
        args.extend(["--resume", session_id]);
    }
    args.push(prompt);
    args
}

/// The arguments every session runs `codex` with: `exec`, streaming JSONL,
/// with no approvals and no sandbox (ADR-0012), on the Model and Effort
/// `harness` sets, if any, reading `CLAUDE.md` where a directory has no
/// `AGENTS.md`. With `resume`, the session with that id continues, given
/// every one of those again, as Codex keeps none of them. The prompt comes
/// last.
pub fn codex_args(harness: &Choice, resume: Option<&str>, prompt: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
    ]
    .map(String::from)
    .to_vec();
    args.extend(harness.codex_args());
    args.extend(["-c".to_string(), CLAUDE_MD_FALLBACK.to_string()]);
    if let Some(session_id) = resume {
        args.extend(["resume".to_string(), session_id.to_string()]);
    }
    args.push(prompt.to_string());
    args
}

/// The config setting that has Codex read `CLAUDE.md` where a directory has
/// no `AGENTS.md`.
const CLAUDE_MD_FALLBACK: &str = r#"project_doc_fallback_filenames=["CLAUDE.md"]"#;

const POLL: Duration = Duration::from_millis(100);

/// How long a session gets to exit after each signal that asks it to stop.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// Stop `child`, a session on `harness`, by its process group: with the
/// signals that ask `harness` to stop, in turn, each given `STOP_GRACE`,
/// then SIGKILL.
fn stop(child: &mut Child, harness: Harness) {
    let group = -(child.id() as libc::pid_t);
    for &signal in stop_signals(harness) {
        // SAFETY: kill has no memory-safety preconditions.
        unsafe { libc::kill(group, signal) };
        let deadline = Instant::now() + STOP_GRACE;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            thread::sleep(POLL);
        }
    }
    // SAFETY: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
    let _ = child.wait();
}

/// The signals that ask a session on `harness` to stop, in the order they
/// are sent: SIGTERM, after SIGINT for Codex, which stops cleanly only on
/// SIGINT, interrupting its turn.
fn stop_signals(harness: Harness) -> &'static [libc::c_int] {
    match harness {
        Harness::Claude => &[libc::SIGTERM],
        Harness::Codex => &[libc::SIGINT, libc::SIGTERM],
    }
}

/// Copy every line of `stream` to `log_file` and print the progress lines it
/// condenses to, until the stream ends.
fn follow(
    kind: &str,
    stream: impl Read,
    log_file: &mut File,
    log: &Path,
    progress: &mut dyn Stream,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_that_loads_a_skill_loads_it_with_codexs_sigil_on_codex() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        for (prompt, first_line) in [
            (
                prompt::fresh(&issue, "main", "issue-7"),
                "$thirdshift-implement https://github.com/acme/widgets/issues/7",
            ),
            (
                prompt::conflict_repair(&issue, "main", "issue-7", "<pr>"),
                "$thirdshift-resolving-merge-conflicts",
            ),
            (
                prompt::review_repair(&issue, "issue-7", "<pr>", "abc123"),
                "$thirdshift-code-review abc123",
            ),
            (
                prompt::resume(&["cargo test"]),
                "Your background work (cargo test) was killed when your turn ended, because \
                 ending the turn ends the session.",
            ),
        ] {
            let codex_prompt = codex_prompt(&prompt);
            assert_eq!(codex_prompt.lines().next(), Some(first_line));
            assert_eq!(codex_prompt[1..], prompt[1..]);
        }
    }
}
