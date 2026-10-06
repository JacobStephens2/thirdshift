//! Test harness: runs the compiled `thirdshift` binary against real git and
//! fake `gh`, `claude` and `codex` executables, and, for email, a stand-in for Resend
//! ([`resend::ResendStandIn`]).
//!
//! Layout of a scenario's temp root:
//!
//! ```text
//! origin.git/        bare repo standing in for github.com/<owner>/<repo>;
//!                    it rejects non-fast-forward pushes, so no rebase or
//!                    force-push can reach it
//! home/              $HOME: .gitconfig with identity, the insteadOf rule and
//!                    auto maintenance off
//! home/.thirdshift/  the User config, config.toml, if the test writes one
//! home/.config/      $XDG_CONFIG_HOME, where an install receipt would be
//! installed/         a copy of thirdshift, if the test runs one to replace it
//! bin/               fake gh, claude and codex, first on PATH
//! tmp/               $TMPDIR, so leftover temp directories are visible
//! work/<repo>/       the launch clone, origin https://github.com/<owner>/<repo>.git
//! gh-state.json      fake GitHub state
//! claude-script.sh   what the fake agent does this test
//! claude-script.sh.<n>  what it does in the n-th session instead, if present
//! claude-script.sh.issue-<i>    what it does in sessions for issue <i>, if
//!                    present, taking precedence over the two above
//! claude-script.sh.issue-<i>.<k>  what it does in the k-th session for issue
//!                    <i>, if present, taking precedence over all the rest
//! claude-calls.json  what the fake agent was asked to do
//! claude-calls.json.after-result.<n>  stream lines the n-th session emits
//!                    after its closing result
//! claude-calls.json.final-message.<n>  the n-th session's final message
//! codex-calls.json   what the fake agent was asked to do on Codex, whose
//!                    sessions take the same scripts
//! codex-calls.json.final-message.<n>, codex-calls.json.error.<n>  the n-th
//!                    Codex session's final message, and the error it
//!                    failed with
//! gh-calls.json      every gh command run, by thirdshift or the fake agent
//! ```

#![allow(dead_code)]

pub mod fakes;
pub mod resend;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

pub const OWNER: &str = "acme";
pub const REPO: &str = "widgets";

/// Every Factory skill, by the name a session finds it under.
pub const FACTORY_SKILLS: [&str; 9] = [
    "thirdshift-implement",
    "thirdshift-code-review",
    "thirdshift-pr",
    "thirdshift-tdd",
    "thirdshift-resolving-merge-conflicts",
    "thirdshift-improve-codebase-architecture",
    "thirdshift-to-spec",
    "thirdshift-to-tickets",
    "thirdshift-codebase-design",
];

/// How long a test waits for a Run to reach a point, such as starting the
/// agent or showing a prompt, before it gives the Run up as hung. A Run that
/// exits without reaching the point fails its test at once, so only a hung
/// Run waits this long: on a busy machine a Run that is getting there can
/// take many times what it takes on an idle one (#200).
pub const WAIT_BOUND: Duration = Duration::from_secs(120);

/// The file in a scenario's root a session touches once it is waiting for
/// the copy of thirdshift its Run was started from to be replaced.
const COPY_IN_USE: &str = "copy-in-use";

/// The file in a scenario's root that says the copy has been replaced.
const COPY_REPLACED: &str = "copy-replaced";

pub struct Scenario {
    /// Deletes the temp root when the scenario is dropped.
    _temp_dir: TempDir,
    /// The temp root's path with symlinks resolved, as git reports it: on
    /// macOS the temp directory is under `/var`, a symlink to `/private/var`.
    root: PathBuf,
}

/// What typing into thirdshift on a terminal does: wait until the terminal
/// shows `prompt`, then type `line` and Enter, or with [`CTRL_C`] press
/// Ctrl-C instead.
pub type Keystrokes<'a> = (&'a str, &'a str);

/// A `line` of [`Keystrokes`] that presses Ctrl-C.
pub const CTRL_C: &str = "\x03";

/// How a command run on a terminal ended.
pub struct TerminalResult {
    /// What it wrote to stdout, which is not the terminal.
    pub stdout: String,
    /// Everything the terminal showed, stderr and the echoed keystrokes,
    /// with the terminal's `\r\n` as `\n` and progress lines unstamped.
    pub stderr: String,
    /// `None` if a signal ended it.
    pub code: Option<i32>,
    /// The User config it left, if any.
    pub user_config: Option<String>,
}

/// An event on an issue's timeline that a Pickup run reads to tell whether
/// the issue has settled.
pub enum TimelineEvent<'a> {
    /// This label was applied.
    Labelled(&'a str),
    SubIssueAdded,
    SubIssueRemoved,
    BlockedByAdded,
    BlockedByRemoved,
}

pub struct RunResult {
    pub stdout: String,
    /// stderr with the time each progress line was printed removed, so it
    /// can be matched exactly: see [`unstamped`].
    pub stderr: String,
    /// stderr as printed, times and all.
    pub stamped_stderr: String,
    pub code: Option<i32>,
}

impl From<Output> for RunResult {
    fn from(output: Output) -> Self {
        let stamped_stderr = String::from_utf8(output.stderr).unwrap();
        RunResult {
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: unstamped(&stamped_stderr),
            stamped_stderr,
            code: output.status.code(),
        }
    }
}

/// A Run started and waited on until its fake agent started, as by
/// [`Scenario::run_until`], which its fake agent's script may still be
/// holding where it is.
pub struct HeldRun {
    child: Child,
    /// Whether the Run had exited by the time the agent was seen to start.
    exited: bool,
}

impl HeldRun {
    /// Send the Run `signal` (e.g. `"INT"`), unless it had already exited.
    pub fn signal(&mut self, signal: &str) {
        // A Run that has exited is past signalling, and its process id may
        // be another process's by now.
        if self.exited {
            return;
        }
        let status = Command::new("kill")
            .args([&format!("-{signal}"), &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
    }

    /// Kill the Run, unless it had already exited, and wait until it is gone.
    /// What it started lives on.
    pub fn kill(&mut self) {
        self.signal("KILL");
        self.child.wait().unwrap();
    }

    /// Wait for the Run to exit, and for everything it started to let go of
    /// its stdout and stderr.
    pub fn finish(self) -> RunResult {
        self.child.wait_with_output().unwrap().into()
    }
}

/// A script in which the agent starts a background task described as
/// `description`, then ends its turn with it still running, so the task is
/// killed after the session's last `result`.
pub fn leaves_running(description: &str) -> String {
    format!(
        r#"
echo '{{"type": "system", "subtype": "task_started", "task_id": "b1", "description": "{description}"}}'
echo '{{"type": "system", "subtype": "task_updated", "task_id": "b1", "patch": {{"status": "killed"}}}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#
    )
}

/// `stderr` with the `HH:MM:SS ` that starts each progress line, after its
/// `thirdshift: `, removed.
pub fn unstamped(stderr: &str) -> String {
    stderr
        .split_inclusive('\n')
        .map(
            |line| match line.strip_prefix("thirdshift: ").and_then(split_stamp) {
                Some((_, message)) => format!("thirdshift: {message}"),
                None => line.to_string(),
            },
        )
        .collect()
}

/// A progress line after its `thirdshift: ` split into the `HH:MM:SS` it
/// starts with and the message after it, if it starts with one. Mirrors
/// `progress::split_stamp`, which the binary keeps to itself.
pub fn split_stamp(unprefixed: &str) -> Option<(&str, &str)> {
    let (time, message) = unprefixed.split_at_checked(8)?;
    let message = message.strip_prefix(' ')?;
    let is_stamp = time.bytes().enumerate().all(|(at, byte)| match at {
        2 | 5 => byte == b':',
        _ => byte.is_ascii_digit(),
    });
    is_stamp.then_some((time, message))
}

/// `stderr` after its first line, which must say the command is starting
/// and when, as `thirdshift: <starting>, <date> <offset>` does for a Run, a
/// Spec run, an Architect run and a Pickup run.
pub fn after_start<'a>(stderr: &'a str, starting: &str) -> &'a str {
    let (first, rest) = stderr.split_once('\n').unwrap_or((stderr, ""));
    assert!(
        first.starts_with(&format!("thirdshift: {starting}, ")),
        "not starting as {starting:?}: {stderr}"
    );
    rest
}

/// The lines that end a Failed run's `stderr` once its cause is given, the
/// line naming its Command log left off.
pub fn before_command_log(stderr: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = stderr.lines().collect();
    let last = lines.pop().unwrap_or_default();
    assert!(
        last.starts_with("thirdshift: command log: "),
        "not ending with the Command log: {stderr}"
    );
    lines
}

impl Scenario {
    /// An origin with one commit on `main`, a launch clone of it with `main`
    /// checked out, and an open issue #7.
    pub fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().canonicalize().unwrap();
        let scenario = Scenario {
            _temp_dir: temp_dir,
            root,
        };
        for dir in ["home", "bin", "tmp", "work"] {
            fs::create_dir_all(scenario.path(dir)).unwrap();
        }
        scenario.write_gitconfig();
        fakes::install(&scenario.path("bin"));
        scenario.write_gh_state(&json!({
            "repo": format!("{OWNER}/{REPO}"),
            "issues": { "7": "OPEN" },
            "prs": [],
        }));
        scenario.agent_does("true");

        git(
            &scenario.path(""),
            &["init", "--bare", "--initial-branch=main", "origin.git"],
        );
        git(
            &scenario.origin_dir(),
            &["config", "receive.denyNonFastForwards", "true"],
        );
        let seed = scenario.path("seed");
        git(
            &scenario.path(""),
            &["clone", &scenario.github_url(), "seed"],
        );
        fs::write(seed.join("README.md"), "widgets\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-m", "Initial commit"]);
        git(&seed, &["push", "origin", "HEAD:main"]);
        fs::remove_dir_all(&seed).unwrap();

        git(
            &scenario.path("work"),
            &["clone", &scenario.github_url(), REPO],
        );
        scenario
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub fn launch_dir(&self) -> PathBuf {
        self.path("work").join(REPO)
    }

    pub fn origin_dir(&self) -> PathBuf {
        self.path("origin.git")
    }

    pub fn github_url(&self) -> String {
        format!("https://github.com/{OWNER}/{REPO}.git")
    }

    pub fn issue_url(&self, number: u32) -> String {
        format!("https://github.com/{OWNER}/{REPO}/issues/{number}")
    }

    /// Script the fake agent: `script` is bash, run with `-e` in the session's
    /// working directory.
    pub fn agent_does(&self, script: &str) {
        fs::write(self.path("claude-script.sh"), script).unwrap();
    }

    /// Script the fake agent's `session`-th session (1-based) differently
    /// from the rest.
    pub fn agent_does_in_session(&self, session: usize, script: &str) {
        fs::write(self.path(&format!("claude-script.sh.{session}")), script).unwrap();
    }

    /// Script the fake agent's sessions for issue `issue`, the one whose
    /// Issue URL their prompt names, whatever order they come in.
    pub fn agent_does_for(&self, issue: u32, script: &str) {
        fs::write(
            self.path(&format!("claude-script.sh.issue-{issue}")),
            script,
        )
        .unwrap();
    }

    /// Script the fake agent's `session`-th session (1-based) for issue
    /// `issue` differently from its others.
    pub fn agent_does_for_in_session(&self, issue: u32, session: usize, script: &str) {
        fs::write(
            self.path(&format!("claude-script.sh.issue-{issue}.{session}")),
            script,
        )
        .unwrap();
    }

    pub fn run(&self, args: &[&str]) -> RunResult {
        self.command(args).output().unwrap().into()
    }

    /// Like [`Scenario::run`], with extra environment variables.
    pub fn run_with_env(&self, args: &[&str], env: &[(&str, &str)]) -> RunResult {
        let mut command = self.command(args);
        command.envs(env.iter().copied());
        command.output().unwrap().into()
    }

    /// Run thirdshift and send it `signal` (e.g. `"INT"`) once the fake agent
    /// has touched the file `started` in the scenario root. Panics with the
    /// Run's stderr if the Run exits without the file there, or if the file
    /// isn't there within [`WAIT_BOUND`]. A Run that exits just after the
    /// file appears gets no signal.
    pub fn run_and_signal(&self, args: &[&str], started: &str, signal: &str) -> RunResult {
        self.run_and_signal_with_env(args, &[], started, signal)
    }

    /// Like [`Scenario::run_and_signal`], with extra environment variables.
    pub fn run_and_signal_with_env(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        started: &str,
        signal: &str,
    ) -> RunResult {
        let mut held = self.run_until(args, env, started);
        held.signal(signal);
        held.finish()
    }

    /// Start thirdshift and return once the fake agent has touched the file
    /// `started` in the scenario root, with the Run still going unless it
    /// exited just after. Panics with the Run's stderr if the Run exits
    /// without the file there, or if the file isn't there within
    /// [`WAIT_BOUND`].
    pub fn run_until(&self, args: &[&str], env: &[(&str, &str)], started: &str) -> HeldRun {
        let mut command = self.command(args);
        command.envs(env.iter().copied());
        self.spawn_until_started(command, started)
    }

    /// Run a copy of thirdshift, `installed/thirdshift` in the scenario root,
    /// as an installed one is run, and call `replace` with the copy's path
    /// once a session whose script has [`Scenario::waits_to_be_replaced`]
    /// reaches it. That session goes on once `replace` has returned, so
    /// whatever the Run starts after it starts with the copy replaced.
    pub fn run_copy_replaced_midway(
        &self,
        args: &[&str],
        replace: impl FnOnce(&Path),
    ) -> RunResult {
        let copy = self.path("installed/thirdshift");
        fs::create_dir_all(copy.parent().unwrap()).unwrap();
        // Copied by `cp`, not by this process: a file open for writing here
        // would be inherited by whatever another test starts meanwhile, and
        // can't be run until that has let go of it ("Text file busy").
        let copied = Command::new("cp")
            .arg(env!("CARGO_BIN_EXE_thirdshift"))
            .arg(&copy)
            .status()
            .unwrap();
        assert!(copied.success());
        let held = self.spawn_until_started(self.command_running(&copy, args), COPY_IN_USE);
        replace(&copy);
        fs::write(self.path(COPY_REPLACED), "").unwrap();
        held.finish()
    }

    /// Bash for a session of a Run started by
    /// [`Scenario::run_copy_replaced_midway`]: it waits up to [`WAIT_BOUND`]
    /// for the copy of thirdshift to be replaced, failing if it never is.
    pub fn waits_to_be_replaced(&self) -> String {
        let looks = WAIT_BOUND.as_millis() / 50;
        format!(
            r#"
touch {root}/{COPY_IN_USE}
for _ in $(seq {looks}); do test -f {root}/{COPY_REPLACED} && break; sleep 0.05; done
test -f {root}/{COPY_REPLACED}
"#,
            root = self.root.display()
        )
    }

    /// Spawn `command`, its stdout and stderr piped, and wait until the fake
    /// agent has touched the file `started` in the scenario root, with the
    /// Run still going unless it exited just after. Panics with the Run's
    /// stderr if the Run exits without the file there, or if the file isn't
    /// there within [`WAIT_BOUND`].
    fn spawn_until_started(&self, mut command: Command, started: &str) -> HeldRun {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + WAIT_BOUND;
        // Whether the Run has exited is read before whether the agent has
        // started, so a Run that starts the agent and exits between the two
        // reads still counts as started.
        let exited = loop {
            let exited = child.try_wait().unwrap().is_some();
            if self.path(started).exists() {
                break exited;
            }
            if exited {
                let result = RunResult::from(child.wait_with_output().unwrap());
                panic!(
                    "the agent never started: the Run exited first; stderr:\n{}",
                    result.stderr
                );
            }
            assert!(Instant::now() < deadline, "the agent never started");
            std::thread::sleep(Duration::from_millis(20));
        };
        HeldRun { child, exited }
    }

    /// Run thirdshift with stdin and stderr on a pseudo-terminal, as from an
    /// interactive shell, typing `keystrokes` in order. stdout is a pipe, so
    /// what it prints there stays apart from the terminal. Panics with what
    /// the terminal shows if the command exits without showing a prompt, or
    /// doesn't show it within [`WAIT_BOUND`].
    pub fn run_on_terminal(
        &self,
        args: &[&str],
        env: &[(&str, &str)],
        keystrokes: &[Keystrokes],
    ) -> TerminalResult {
        use std::io::{Read, Write};
        use std::os::fd::{FromRawFd, OwnedFd};
        use std::os::unix::process::CommandExt;
        use std::sync::{Arc, Mutex};

        let (mut master, slave) = {
            let (mut master, mut slave) = (0, 0);
            let opened = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
            unsafe { (fs::File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
        };
        let mut command = self.command(args);
        command
            .envs(env.iter().copied())
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .stdout(Stdio::piped());
        // The terminal becomes the child's controlling terminal, so Ctrl-C
        // on it sends SIGINT, as in a shell.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        // Dropping the command closes this process's copies of the terminal,
        // so reading it ends once the child has gone.
        drop(command);

        let shown = Arc::new(Mutex::new(Vec::new()));
        let reader = {
            let mut master = master.try_clone().unwrap();
            let shown = Arc::clone(&shown);
            std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                // Linux ends a pseudo-terminal with EIO rather than EOF.
                while let Ok(read @ 1..) = master.read(&mut buffer) {
                    shown.lock().unwrap().extend_from_slice(&buffer[..read]);
                }
            })
        };
        let stdout = {
            let mut stdout = child.stdout.take().unwrap();
            std::thread::spawn(move || {
                let mut text = String::new();
                stdout.read_to_string(&mut text).unwrap();
                text
            })
        };
        let shown_text = || String::from_utf8_lossy(&shown.lock().unwrap()).replace("\r\n", "\n");
        let mut reader = Some(reader);
        let mut seen = 0;
        for (prompt, line) in keystrokes {
            let deadline = Instant::now() + WAIT_BOUND;
            loop {
                // Once the Run has exited, the terminal is read to its end
                // before it is searched, so nothing the Run showed is missed.
                let exited = child.try_wait().unwrap().is_some();
                if exited && let Some(reader) = reader.take() {
                    reader.join().unwrap();
                }
                let text = shown_text();
                if let Some(at) = text[seen..].find(prompt) {
                    seen += at + prompt.len();
                    break;
                }
                assert!(
                    !exited,
                    "the terminal never showed {prompt:?}: the Run exited first; it shows:\n{text}"
                );
                assert!(
                    Instant::now() < deadline,
                    "the terminal never showed {prompt:?}; it shows:\n{text}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            if *line == CTRL_C {
                master.write_all(line.as_bytes()).unwrap();
            } else {
                master.write_all(format!("{line}\n").as_bytes()).unwrap();
            }
        }
        let status = child.wait().unwrap();
        let stdout = stdout.join().unwrap();
        if let Some(reader) = reader {
            reader.join().unwrap();
        }
        TerminalResult {
            stdout,
            stderr: unstamped(&shown_text()),
            code: status.code(),
            user_config: fs::read_to_string(self.path("home/.thirdshift/config.toml")).ok(),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        self.command_running(Path::new(env!("CARGO_BIN_EXE_thirdshift")), args)
    }

    /// Like [`Scenario::command`], running the thirdshift at `executable`.
    fn command_running(&self, executable: &Path, args: &[&str]) -> Command {
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(self.launch_dir())
            .env_clear()
            .env("PATH", path)
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("home/.config"))
            .env("TMPDIR", self.path("tmp"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("FAKE_GH_STATE", self.path("gh-state.json"))
            .env("FAKE_CLAUDE_SCRIPT", self.path("claude-script.sh"))
            .env("FAKE_CLAUDE_RECORD", self.path("claude-calls.json"))
            .env("FAKE_CODEX_RECORD", self.path("codex-calls.json"))
            .env("FAKE_AGY_RECORD", self.path("agy-calls.json"))
            .env("FAKE_AGY_CHECK_RECORD", self.path("agy-checks.json"))
            .env("FAKE_GH_RECORD", self.path("gh-calls.json"))
            // Seconds of waiting for CI become milliseconds. Each poll starts
            // the fake gh; at 100ms the grace period holds about three reads,
            // enough for the absent state.
            .env("THIRDSHIFT_CI_GRACE_MS", "300")
            .env("THIRDSHIFT_POLL_MS", "100");
        command
    }

    /// Write `toml` as the User config, `~/.thirdshift/config.toml` under the
    /// scenario's `$HOME`, and return its path.
    pub fn user_config_is(&self, toml: &str) -> PathBuf {
        let dir = self.path("home/.thirdshift");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, toml).unwrap();
        path
    }

    /// Write `toml` as the Credentials, `~/.thirdshift/credentials.toml`,
    /// readable only by the user, and return its path.
    pub fn credentials_are(&self, toml: &str) -> PathBuf {
        let dir = self.path("home/.thirdshift");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("credentials.toml");
        fs::write(&path, toml).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    /// The Credentials, `~/.thirdshift/credentials.toml`, if there are any.
    pub fn credentials(&self) -> Option<String> {
        fs::read_to_string(self.path("home/.thirdshift/credentials.toml")).ok()
    }

    /// Set issue `number`'s state on the fake GitHub: `"OPEN"` or `"CLOSED"`.
    pub fn issue_is(&self, number: u32, state: &str) {
        let mut gh = self.gh_state();
        gh["issues"][number.to_string()] = json!(state);
        self.write_gh_state(&gh);
    }

    /// Make issue `spec` a Spec on the fake GitHub: `tickets` are its
    /// sub-issues, in order, each with the issues it is blocked by. Every
    /// issue named that the fake GitHub doesn't know yet is open.
    pub fn spec_has_tickets(&self, spec: u32, tickets: &[(u32, &[u32])]) {
        let mut gh = self.gh_state();
        let named = std::iter::once(spec).chain(tickets.iter().flat_map(|(ticket, blockers)| {
            std::iter::once(*ticket).chain(blockers.iter().copied())
        }));
        for number in named {
            let issue = &mut gh["issues"][number.to_string()];
            if issue.is_null() {
                *issue = json!("OPEN");
            }
        }
        gh["sub_issues"][spec.to_string()] =
            json!(tickets.iter().map(|(ticket, _)| ticket).collect::<Vec<_>>());
        for (ticket, blockers) in tickets {
            gh["blocked_by"][ticket.to_string()] = json!(blockers);
        }
        self.write_gh_state(&gh);
    }

    /// Make issue `number` blocked by `blockers` on the fake GitHub, by its
    /// "blocked by" links. A blocker the fake GitHub doesn't know yet is
    /// open.
    pub fn issue_blocked_by(&self, number: u32, blockers: &[u32]) {
        let mut gh = self.gh_state();
        gh["blocked_by"][number.to_string()] = json!(blockers);
        self.write_gh_state(&gh);
    }

    /// Set issue `number`'s timeline on the fake GitHub to `events`, oldest
    /// first, each with how many minutes ago it happened.
    pub fn issue_timeline(&self, number: u32, events: &[(TimelineEvent, i64)]) {
        let events: Vec<Value> = events
            .iter()
            .map(|(event, minutes_ago)| {
                let at = chrono::Utc::now() - chrono::TimeDelta::minutes(*minutes_ago);
                let at = at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
                let event = match event {
                    TimelineEvent::Labelled(label) => {
                        return json!({"event": "labeled", "label": label, "at": at});
                    }
                    TimelineEvent::SubIssueAdded => "sub_issue_added",
                    TimelineEvent::SubIssueRemoved => "sub_issue_removed",
                    TimelineEvent::BlockedByAdded => "blocked_by_added",
                    TimelineEvent::BlockedByRemoved => "blocked_by_removed",
                };
                json!({"event": event, "at": at})
            })
            .collect();
        let mut gh = self.gh_state();
        gh["timeline"][number.to_string()] = json!(events);
        self.write_gh_state(&gh);
    }

    /// Give issue `number` the labels `labels` on the fake GitHub.
    pub fn issue_labelled(&self, number: u32, labels: &[&str]) {
        let mut gh = self.gh_state();
        gh["labels"][number.to_string()] = json!(labels);
        self.write_gh_state(&gh);
    }

    /// Set when issue `number` was created on the fake GitHub, as GitHub
    /// writes a time: `2026-10-01T12:00:00Z`.
    pub fn issue_created(&self, number: u32, time: &str) {
        let mut gh = self.gh_state();
        gh["created"][number.to_string()] = json!(time);
        self.write_gh_state(&gh);
    }

    /// The labels of issue `number` on the fake GitHub.
    pub fn issue_labels(&self, number: u32) -> Vec<String> {
        let labels = &self.gh_state()["labels"][number.to_string()];
        let labels = labels.as_array().into_iter().flatten();
        labels
            .map(|label| label.as_str().unwrap().to_string())
            .collect()
    }

    /// Give the repository the labels `labels` on the fake GitHub.
    pub fn repo_has_labels(&self, labels: &[&str]) {
        let mut gh = self.gh_state();
        gh["repo_labels"] = json!(labels);
        self.write_gh_state(&gh);
    }

    /// The repository's labels on the fake GitHub.
    pub fn repo_labels(&self) -> Vec<String> {
        let labels = &self.gh_state()["repo_labels"];
        let labels = labels.as_array().into_iter().flatten();
        labels
            .map(|label| label.as_str().unwrap().to_string())
            .collect()
    }

    /// Give issue `number` the title `title` on the fake GitHub.
    pub fn issue_titled(&self, number: u32, title: &str) {
        let mut gh = self.gh_state();
        gh["titles"][number.to_string()] = json!(title);
        self.write_gh_state(&gh);
    }

    /// Set the public email of the profile `gh api user` answers with: an
    /// address, or `None` for a private one.
    pub fn github_email_is(&self, email: Option<&str>) {
        let mut gh = self.gh_state();
        gh["user_email"] = json!(email);
        self.write_gh_state(&gh);
    }

    /// Make every `gh api user` call fail.
    pub fn github_profile_fails(&self) {
        self.gh_fails("api user");
    }

    /// Make every `gh <call>` fail, `call` being the command's first two
    /// arguments, e.g. `label list`, or more of them, e.g. `api --method
    /// DELETE`.
    pub fn gh_fails(&self, call: &str) {
        let mut gh = self.gh_state();
        let failing = gh.as_object_mut().unwrap().entry("failing");
        failing
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!(call));
        self.write_gh_state(&gh);
    }

    /// Set the global git `user.email`, or with `None` unset it.
    pub fn git_email_is(&self, email: Option<&str>) {
        let change = match email {
            Some(email) => ["user.email", email],
            None => ["--unset", "user.email"],
        };
        git(
            &self.path("home"),
            &[&["config", "--global"][..], &change].concat(),
        );
    }

    pub fn gh_state(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.path("gh-state.json")).unwrap()).unwrap()
    }

    pub fn write_gh_state(&self, state: &Value) {
        fs::write(
            self.path("gh-state.json"),
            serde_json::to_string_pretty(state).unwrap(),
        )
        .unwrap();
    }

    /// Every call the fake agent received, in order.
    pub fn claude_calls(&self) -> Vec<Value> {
        match fs::read_to_string(self.path("claude-calls.json")) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => Vec::new(),
        }
    }

    /// Every call the fake agent received on Codex, in order.
    pub fn codex_calls(&self) -> Vec<Value> {
        match fs::read_to_string(self.path("codex-calls.json")) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => Vec::new(),
        }
    }

    pub fn agy_calls(&self) -> Vec<Value> {
        self.agy_records("agy-calls.json")
    }

    pub fn agy_checks(&self) -> Vec<Value> {
        self.agy_records("agy-checks.json")
    }

    fn agy_records(&self, file: &str) -> Vec<Value> {
        fs::read_to_string(self.path(file))
            .ok()
            .map(|text| serde_json::from_str(&text).unwrap())
            .unwrap_or_default()
    }

    /// Assert every `claude` call found every Factory skill, by its
    /// `thirdshift-<skill>` name, in its worktree's `.claude/skills/`.
    pub fn assert_every_session_found_the_factory_skills(&self) {
        assert_found_the_factory_skills(self.claude_calls());
    }

    /// Assert every `codex` call found every Factory skill, by its
    /// `thirdshift-<skill>` name, in its worktree's `.agents/skills/`.
    pub fn assert_every_codex_session_found_the_factory_skills(&self) {
        assert_found_the_factory_skills(self.codex_calls());
    }

    /// The prompt of the first `claude` call.
    pub fn first_prompt(&self) -> String {
        self.claude_calls()[0]["prompt"]
            .as_str()
            .expect("claude got no prompt")
            .to_string()
    }

    /// The argv of every `gh` call, in order.
    pub fn gh_calls(&self) -> Vec<Vec<String>> {
        match fs::read_to_string(self.path("gh-calls.json")) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => Vec::new(),
        }
    }

    /// Every `gh <command> <subcommand>` call made, e.g. `gh pr merge`.
    pub fn gh_calls_of(&self, command: &str, subcommand: &str) -> Vec<Vec<String>> {
        self.gh_calls()
            .into_iter()
            .filter(|call| call.starts_with(&[command.to_string(), subcommand.to_string()]))
            .collect()
    }

    /// Push `branch` to origin: `from` plus one commit per subject in
    /// `commits`, oldest first.
    pub fn origin_has_branch(&self, branch: &str, from: &str, commits: &[&str]) {
        self.push_from_seed(branch, |seed| {
            git(
                seed,
                &["checkout", "-q", "-b", branch, &format!("origin/{from}")],
            );
            for (i, subject) in commits.iter().enumerate() {
                fs::write(seed.join(format!("{branch}-{i}.txt")), subject).unwrap();
                git(seed, &["add", "."]);
                git(seed, &["commit", "-q", "-m", subject]);
            }
        });
    }

    /// Push one commit to `branch` on origin that writes `contents` to
    /// `file`, as another machine might while the Launch directory isn't
    /// looking.
    pub fn origin_has_commit(&self, branch: &str, file: &str, contents: &str, subject: &str) {
        self.push_from_seed(branch, |seed| {
            git(seed, &["checkout", "-q", branch]);
            fs::write(seed.join(file), contents).unwrap();
            git(seed, &["add", file]);
            git(seed, &["commit", "-q", "-m", subject]);
        });
    }

    /// Clone origin into a scratch `seed`, let `commit` make commits on
    /// `branch` there, push `branch` and delete the clone.
    fn push_from_seed(&self, branch: &str, commit: impl FnOnce(&Path)) {
        let seed = self.path("seed");
        git(&self.path(""), &["clone", "-q", &self.github_url(), "seed"]);
        commit(&seed);
        git(&seed, &["push", "-q", "origin", branch]);
        fs::remove_dir_all(&seed).unwrap();
    }

    /// Add a PR from `head` into `base` in `state` (`OPEN`, `CLOSED` or
    /// `MERGED`) to the fake GitHub and return its URL.
    pub fn github_has_pr(&self, head: &str, base: &str, state: &str) -> String {
        let mut gh = self.gh_state();
        let prs = gh["prs"].as_array_mut().unwrap();
        let number = prs.len() + 1;
        let url = format!("https://github.com/{OWNER}/{REPO}/pull/{number}");
        prs.push(json!({
            "number": number,
            "url": url,
            "head": head,
            "base": base,
            "state": state,
            "isDraft": false,
            "title": format!("Work on {head}"),
            "body": "",
        }));
        self.write_gh_state(&gh);
        url
    }

    /// Commit subjects on `branch` in the origin repo, newest first, or `None`
    /// if the branch doesn't exist there.
    pub fn origin_log(&self, branch: &str) -> Option<Vec<String>> {
        try_git(
            &self.origin_dir(),
            &["log", "--format=%s", &format!("refs/heads/{branch}")],
        )
        .map(|log| log.lines().map(String::from).collect())
    }

    /// The contents of `file` on `branch` in the origin repo, or `None` if
    /// either doesn't exist there.
    pub fn origin_file(&self, branch: &str, file: &str) -> Option<String> {
        try_git(
            &self.origin_dir(),
            &["show", &format!("refs/heads/{branch}:{file}")],
        )
    }

    /// Fetch origin in the launch clone and check out `branch` there.
    pub fn launch_checks_out(&self, branch: &str) {
        self.launch_git(&["fetch", "-q", "origin"]);
        self.launch_git(&["checkout", "-q", branch]);
    }

    /// Output of a git command in the origin repo, panicking on failure.
    pub fn origin_git(&self, args: &[&str]) -> String {
        git(&self.origin_dir(), args)
    }

    /// Assert the Run left no worktree, local `branch` or temp directory.
    pub fn assert_cleaned_up(&self, branch: &str) {
        assert_eq!(self.entries("work"), vec![REPO]);
        assert_eq!(
            self.launch_git(&["worktree", "list", "--porcelain"])
                .matches("worktree ")
                .count(),
            1
        );
        assert_eq!(self.launch_git(&["branch", "--list", branch]), "");
        assert_eq!(self.entries("tmp"), Vec::<String>::new());
    }

    /// Install `script` as the `name` hook (e.g. `"pre-push"`) of the git
    /// repository at `repo`, such as the launch clone or `origin.git`.
    pub fn repo_has_hook(&self, repo: &Path, name: &str, script: &str) {
        let hooks = if repo.join(".git").is_dir() {
            repo.join(".git/hooks")
        } else {
            repo.join("hooks")
        };
        fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join(name);
        fs::write(&hook, script).unwrap();
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Give the launch clone a `pre-push` hook that says "hook says no" and
    /// rejects every push, as a target repo's local hook might.
    pub fn launch_has_rejecting_pre_push_hook(&self) {
        self.repo_has_hook(
            &self.launch_dir(),
            "pre-push",
            "#!/bin/sh\necho \"hook says no\"\nexit 1\n",
        );
    }

    /// Output of a git command in the launch clone.
    pub fn launch_git(&self, args: &[&str]) -> String {
        git(&self.launch_dir(), args)
    }

    /// Commit `file` with `contents` on the launch clone's current branch,
    /// without pushing it.
    pub fn commit_locally(&self, file: &str, contents: &str, message: &str) {
        fs::write(self.launch_dir().join(file), contents).unwrap();
        self.launch_git(&["add", file]);
        self.launch_git(&["commit", "-q", "-m", message]);
    }

    /// Assert that a Run was rejected with `message` before it created
    /// anything: no worktree, no temp directory, no agent session.
    pub fn assert_rejected_before_any_work(&self, result: &RunResult, message: &str) {
        assert_ne!(result.code, Some(0), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains(message),
            "expected {message:?} in stderr: {}",
            result.stderr
        );
        assert_eq!(result.stdout, "");
        assert_eq!(self.entries("work"), vec![REPO]);
        assert_eq!(
            self.launch_git(&["worktree", "list", "--porcelain"])
                .matches("worktree ")
                .count(),
            1
        );
        assert_eq!(self.entries("tmp"), Vec::<String>::new());
        assert!(self.claude_calls().is_empty(), "claude was invoked");
    }

    /// Names of the files and directories directly inside `relative`.
    pub fn entries(&self, relative: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(relative))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// Point the launch clone's origin at `url`, another spelling of the
    /// GitHub URL, which git also redirects to the bare repo.
    pub fn set_origin_url(&self, url: &str) {
        let rule = format!("url.{}.insteadOf", self.origin_dir().display());
        self.launch_git(&["config", "--global", "--add", &rule, url]);
        self.launch_git(&["config", "remote.origin.url", url]);
    }

    /// Write the gitconfig every git command in the scenario reads: the
    /// identity, the default branch, the insteadOf rule, and auto maintenance
    /// off. From 2.47 git detaches auto maintenance into the background after
    /// a commit or a push, where it can still be writing into `.git/objects`
    /// once the command has returned: a script that then clones the origin
    /// again can find an object gone mid-copy, and one that deletes its
    /// temporary clone with `rm -rf` can fail with `Directory not empty`
    /// (#263). `gc.auto` says the same to a git too old to know
    /// `maintenance.auto`.
    fn write_gitconfig(&self) {
        let config = format!(
            "[user]\n\tname = Test Runner\n\temail = runner@example.com\n\
             [init]\n\tdefaultBranch = main\n\
             [maintenance]\n\tauto = false\n\
             [gc]\n\tauto = 0\n\
             [url \"{origin}\"]\n\tinsteadOf = {github}\n",
            origin = self.origin_dir().display(),
            github = self.github_url(),
        );
        fs::write(self.path("home/.gitconfig"), config).unwrap();
    }
}

/// Run git in `dir` with the scenario's config and return stdout, panicking on
/// failure. `dir` must be inside a scenario root.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Like `git`, but `None` if git fails.
fn try_git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = git_output(dir, args);
    if !output.status.success() {
        eprintln!("git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        return None;
    }
    Some(String::from_utf8(output.stdout).unwrap())
}

fn git_output(dir: &Path, args: &[&str]) -> Output {
    let home = dir
        .ancestors()
        .find(|ancestor| ancestor.join("home/.gitconfig").exists())
        .expect("git() called outside a scenario")
        .join("home");
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

/// Assert each of `calls` found every Factory skill, by its
/// `thirdshift-<skill>` name, in its worktree's project skills.
fn assert_found_the_factory_skills(calls: Vec<Value>) {
    assert!(!calls.is_empty(), "no session ran");
    for call in calls {
        for skill in FACTORY_SKILLS {
            let skill_md = call["skill_files"][format!("{skill}/SKILL.md")].as_str();
            assert!(
                skill_md.is_some_and(|text| text.contains(&format!("name: {skill}\n"))),
                "the session in {} with the prompt {} found no {skill}",
                call["cwd"],
                call["prompt"]
            );
        }
    }
}
