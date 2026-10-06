//! Headless agent sessions on the chosen Harness, and their logs.

use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

#[cfg(test)]
pub(crate) use crate::harness::claude::claude_args;
#[cfg(test)]
pub(crate) use crate::harness::codex::{codex_args, codex_prompt};
use crate::harness::{Adapter, Choice, Invocation};
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
    /// How each session is run, and where progress lines go. Held, not
    /// passed to each call as the other seams' are, since the steps reach
    /// it through the `&Sessions` they are given.
    outside: RefCell<Box<dyn Outside>>,
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
        let adapter = harness.harness.adapter();
        if let Err(error) = skills::link_into(worktree, adapter)
            .and_then(|()| adapter.link_instruction_fallback(worktree))
        {
            return (Err(error), None);
        }
        let on_machine = OnMachine {
            worktree: worktree.to_path_buf(),
            harness: harness.clone(),
            adapter,
        };
        let (taken, log) = Sessions::taking(logs, Box::new(on_machine), steps);
        (taken, log.filter(|log| log.exists()))
    }

    /// [`Sessions::within`], its sessions run through `outside`, once the
    /// Factory skills are linked. The log it returns is the most recent
    /// session's, whether or not the session created it.
    fn taking<T>(
        logs: &'a Logs,
        outside: Box<dyn Outside>,
        steps: impl FnOnce(&Self) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>) {
        let sessions = Sessions {
            logs,
            outside: RefCell::new(outside),
            last_log: RefCell::default(),
            endings_with_killed_work: RefCell::default(),
        };
        let taken = steps(&sessions);
        let log = sessions.last_log.into_inner();
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
        if let (false, Some(session_id)) = (ended.killed.is_empty(), &ended.session_id) {
            let resume_prompt = prompt::resume(&ended.killed_work());
            self.step(format!(
                "{kind}: background work was killed as the session ended; resuming it once"
            ));
            ended = self.start(&format!("{kind}-resume"), Some(session_id), &resume_prompt)?;
            ended_last = "Resume";
        }
        if !ended.killed.is_empty() {
            let ending = ending_with(&ended.killed_work());
            self.step(format!(
                "{kind}: the {ended_last} {ending}; carrying on, as it may have been abandoned"
            ));
            self.endings_with_killed_work
                .borrow_mut()
                .push(format!("{kind} session {ending}"));
        }
        Ok(ended.final_message)
    }

    /// Run one session as `kind`, logged under its own path, which becomes
    /// the last log.
    fn start(&self, kind: &str, resume: Option<&str>, prompt: &str) -> Result<Ended> {
        let log = self.logs.path(kind);
        self.step(format!("logging the session to {}", log.display()));
        *self.last_log.borrow_mut() = Some(log.clone());
        self.outside
            .borrow_mut()
            .run_session(kind, resume, prompt, &log)
    }

    /// Hand on the progress line `line`.
    fn step(&self, line: String) {
        self.outside.borrow_mut().step(line);
    }
}

/// How the Resume rules run a session, and where their progress lines go.
trait Outside {
    /// Run a session given `prompt`, labelled as `kind`, continuing the
    /// session with id `resume`, if any, its stream logged to `log`. Returns
    /// what it ended with once it has exited cleanly, its turn not failed.
    fn run_session(
        &mut self,
        kind: &str,
        resume: Option<&str>,
        prompt: &str,
        log: &Path,
    ) -> Result<Ended>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// What a session ended with, as its stream showed it.
struct Ended {
    /// Its id, which a Resume continues.
    session_id: Option<String>,
    /// Descriptions of the background work killed as it ended.
    killed: Vec<String>,
    /// What the agent said as it ended its last turn, if anything.
    final_message: Option<String>,
}

impl Ended {
    /// The descriptions of the killed background work, borrowed.
    fn killed_work(&self) -> Vec<&str> {
        self.killed.iter().map(String::as_str).collect()
    }
}

/// Sessions run by the Command's `harness`'s CLI in `worktree`, where it
/// finds the Factory skills, and progress lines printed on stderr.
struct OnMachine {
    worktree: PathBuf,
    harness: Choice,
    adapter: &'static dyn Adapter,
}

impl Outside for OnMachine {
    fn run_session(
        &mut self,
        kind: &str,
        resume: Option<&str>,
        prompt: &str,
        log: &Path,
    ) -> Result<Ended> {
        let invocation = self.adapter.session(&self.harness, resume, prompt);
        let stream = self.adapter.stream(&self.worktree, prompt);
        let stream = run(kind, self.adapter, &self.worktree, invocation, log, stream)?;
        Ok(Ended {
            session_id: stream.session_id().map(String::from),
            killed: stream
                .killed_background_work()
                .into_iter()
                .map(String::from)
                .collect(),
            final_message: stream.final_message().map(String::from),
        })
    }

    fn step(&mut self, line: String) {
        progress::step(line);
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

/// Run the adapter's CLI with its invocation in
/// `worktree`, where it finds the Factory skills, streaming its output to
/// `log` and condensing it through `stream` to progress lines on stderr,
/// each labelled `kind`. Returns what the stream showed once the CLI has
/// exited cleanly, its turn not failed. Otherwise fails with the error the
/// stream gave, if any. An interrupt stops the session, as [`Adapter::stop`] does,
/// and fails with `interrupted`.
fn run(
    kind: &str,
    adapter: &'static dyn Adapter,
    worktree: &Path,
    invocation: Invocation,
    log: &Path,
    mut stream: Box<dyn Stream>,
) -> Result<Box<dyn Stream>> {
    let cli = adapter.name();
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut log_file =
        File::create(log).with_context(|| format!("could not create {}", log.display()))?;
    let started = Instant::now();
    let mut child = Command::new(cli)
        .args(&invocation.args)
        .envs(adapter.environment().iter().copied())
        .current_dir(worktree)
        .stdin(if invocation.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
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
    if let Some(input) = invocation.stdin
        && let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(input.as_bytes())
    {
        adapter.stop(&mut child);
        let _ = follower.join();
        return Err(error).with_context(|| format!("could not write the prompt to {cli}"));
    }
    let status = loop {
        if interrupt::requested() {
            adapter.stop(&mut child);
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

    let stream = adapter.read_after_exit(worktree, stream);
    for warning in stream.warnings() {
        progress::step(format_args!("{kind}: {warning}"));
    }
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

const POLL: Duration = Duration::from_millis(100);

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
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::*;

    /// What the rules asked of the scripted [`Outside`], in order.
    #[derive(Debug, PartialEq)]
    enum Call {
        /// A session run, as `kind`, continuing `resume`, logged to `log`.
        Session {
            kind: String,
            resume: Option<String>,
            prompt: String,
            log: PathBuf,
        },
        /// A progress line handed on.
        Step(String),
    }

    /// Sessions that end as scripted, in turn, recording every call in
    /// `calls`.
    struct Scripted {
        endings: VecDeque<Result<Ended>>,
        calls: Rc<RefCell<Vec<Call>>>,
    }

    impl Outside for Scripted {
        fn run_session(
            &mut self,
            kind: &str,
            resume: Option<&str>,
            prompt: &str,
            log: &Path,
        ) -> Result<Ended> {
            self.calls.borrow_mut().push(Call::Session {
                kind: kind.to_string(),
                resume: resume.map(String::from),
                prompt: prompt.to_string(),
                log: log.to_path_buf(),
            });
            self.endings
                .pop_front()
                .expect("a session was run that was not scripted")
        }

        fn step(&mut self, line: String) {
            self.calls.borrow_mut().push(Call::Step(line));
        }
    }

    /// A session ending with id `session_id`, if any, with `killed` work
    /// and the final message `said`.
    fn ended(session_id: Option<&str>, killed: &[&str], said: &str) -> Result<Ended> {
        Ok(Ended {
            session_id: session_id.map(String::from),
            killed: killed.iter().map(|work| work.to_string()).collect(),
            final_message: Some(said.to_string()),
        })
    }

    /// The logs of the Run on issue 7, under `/logs`.
    fn logs() -> Logs {
        Logs {
            name: "7".to_string(),
            dir: PathBuf::from("/logs/sessions"),
        }
    }

    /// Take `steps` through sessions that end as `endings` script them.
    /// Returns what the steps came to, the last log, and every call made.
    fn take<T>(
        endings: Vec<Result<Ended>>,
        steps: impl FnOnce(&Sessions) -> Result<T>,
    ) -> (Result<T>, Option<PathBuf>, Vec<Call>) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let scripted = Scripted {
            endings: endings.into(),
            calls: Rc::clone(&calls),
        };
        let logs = logs();
        let (taken, log) = Sessions::taking(&logs, Box::new(scripted), steps);
        let calls = calls.take();
        (taken, log, calls)
    }

    /// The call that runs a session as `kind`, logged under its own path.
    fn session(kind: &str, resume: Option<&str>, prompt: &str) -> Call {
        Call::Session {
            kind: kind.to_string(),
            resume: resume.map(String::from),
            prompt: prompt.to_string(),
            log: logs().path(kind),
        }
    }

    /// The progress line saying where a session as `kind` is logged.
    fn logging(kind: &str) -> Call {
        Call::Step(format!(
            "logging the session to {}",
            logs().path(kind).display()
        ))
    }

    /// The sessions among `calls`.
    fn sessions_run(calls: &[Call]) -> Vec<&Call> {
        calls
            .iter()
            .filter(|call| matches!(call, Call::Session { .. }))
            .collect()
    }

    /// How a session that left `cargo test` running is said to have ended.
    const TESTS_KILLED: &str =
        "ended with a background task still running (cargo test), which was killed";

    #[test]
    fn a_clean_session_runs_once_and_returns_its_final_message() {
        let (taken, log, calls) = take(vec![ended(Some("s1"), &[], "done")], |sessions| {
            sessions.run_to_final_message("implement", "do it")
        });

        assert_eq!(taken.unwrap(), Some("done".to_string()));
        assert_eq!(
            calls,
            [logging("implement"), session("implement", None, "do it")]
        );
        assert_eq!(log, Some(logs().path("implement")));
    }

    #[test]
    fn a_session_whose_background_work_was_killed_is_resumed_once_on_its_id() {
        let (taken, log, calls) = take(
            vec![
                ended(Some("s1"), &["cargo test"], "waiting"),
                ended(Some("s1"), &[], "done"),
            ],
            |sessions| sessions.run_to_final_message("implement", "do it"),
        );

        assert_eq!(taken.unwrap(), Some("done".to_string()));
        assert_eq!(
            calls,
            [
                logging("implement"),
                session("implement", None, "do it"),
                Call::Step(
                    "implement: background work was killed as the session ended; resuming it once"
                        .to_string()
                ),
                logging("implement-resume"),
                session(
                    "implement-resume",
                    Some("s1"),
                    &prompt::resume(&["cargo test"])
                ),
            ]
        );
        assert_eq!(log, Some(logs().path("implement-resume")));
    }

    #[test]
    fn a_resume_that_ends_the_same_way_gets_no_second_and_a_later_failure_names_the_work() {
        let (taken, log, calls) = take(
            vec![
                ended(Some("s1"), &["cargo test"], "waiting"),
                ended(Some("s1"), &["cargo test"], "still waiting"),
            ],
            |sessions| -> Result<()> {
                assert_eq!(
                    sessions.run_to_final_message("implement", "do it").unwrap(),
                    Some("still waiting".to_string())
                );
                bail!("no PR found")
            },
        );

        assert_eq!(sessions_run(&calls).len(), 2, "one Resume, never two");
        assert_eq!(
            calls.last(),
            Some(&Call::Step(format!(
                "implement: the Resume {TESTS_KILLED}; carrying on, as it may have been abandoned"
            )))
        );
        assert_eq!(
            format!("{:#}", taken.unwrap_err()),
            format!("implement session {TESTS_KILLED}, and a later step failed: no PR found")
        );
        assert_eq!(log, Some(logs().path("implement-resume")));
    }

    #[test]
    fn a_session_with_no_id_to_resume_is_not_resumed_and_carries_on() {
        let (taken, log, calls) = take(
            vec![ended(None, &["cargo test", "npm test"], "waiting")],
            |sessions| sessions.run("implement", "do it"),
        );

        taken.unwrap();
        assert_eq!(
            calls,
            [
                logging("implement"),
                session("implement", None, "do it"),
                Call::Step(
                    "implement: the session ended with background tasks still running \
                     (cargo test; npm test), which were killed; carrying on, as it may have \
                     been abandoned"
                        .to_string()
                ),
            ]
        );
        assert_eq!(log, Some(logs().path("implement")));
    }

    #[test]
    fn a_failing_session_fails_unchanged_without_a_resume_or_an_ending() {
        let (taken, log, calls) = take(vec![Err(anyhow!("claude exited 3"))], |sessions| {
            let failed = sessions.run("implement", "do it").unwrap_err();
            assert_eq!(format!("{failed:#}"), "claude exited 3");
            Err::<(), _>(failed)
        });

        // No ending was recorded, so the failure names no killed work.
        assert_eq!(format!("{:#}", taken.unwrap_err()), "claude exited 3");
        assert_eq!(
            calls,
            [logging("implement"), session("implement", None, "do it")]
        );
        // The failing session's log is still the last log.
        assert_eq!(log, Some(logs().path("implement")));
    }

    #[test]
    fn steps_that_succeed_after_killed_work_come_to_what_they_came_to() {
        let (taken, _, _) = take(vec![ended(None, &["cargo test"], "waiting")], |sessions| {
            sessions.run("implement", "do it")?;
            Ok(42)
        });

        assert_eq!(taken.unwrap(), 42);
    }

    #[test]
    fn every_session_that_left_killed_work_is_named_oldest_first() {
        let (taken, _, calls) = take(
            vec![
                ended(None, &["cargo test"], "waiting"),
                ended(None, &["npm test"], "waiting"),
            ],
            |sessions| -> Result<()> {
                sessions.run("implement", "do it")?;
                sessions.run("repair-1", "fix it")?;
                bail!("CI failed")
            },
        );

        assert_eq!(sessions_run(&calls).len(), 2);
        assert_eq!(
            format!("{:#}", taken.unwrap_err()),
            format!(
                "implement session {TESTS_KILLED}; repair-1 session ended with a background \
                 task still running (npm test), which was killed, and a later step failed: \
                 CI failed"
            )
        );
    }

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
