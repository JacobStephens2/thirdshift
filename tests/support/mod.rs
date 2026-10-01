//! Test harness: runs the compiled `thirdshift` binary against real git and
//! fake `gh` and `claude` executables, and, for email, a stand-in for Resend
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
//! bin/               fake gh and claude, first on PATH
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
//! gh-calls.json      every gh command run, by thirdshift or the fake agent
//! ```

#![allow(dead_code)]

pub mod fakes;
pub mod resend;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

pub const OWNER: &str = "acme";
pub const REPO: &str = "widgets";

/// How long a test waits for a Run to reach a point, such as starting the
/// agent or showing a prompt, before it gives the Run up as hung. A Run that
/// exits without reaching the point fails its test at once, so only a hung
/// Run waits this long: on a busy machine a Run that is getting there can
/// take many times what it takes on an idle one (#200).
pub const WAIT_BOUND: Duration = Duration::from_secs(120);

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
        let mut child = self
            .command(args)
            .envs(env.iter().copied())
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
        // A Run that has exited is past signalling, and its process id may
        // be another process's by now.
        if !exited {
            let status = Command::new("kill")
                .args([&format!("-{signal}"), &child.id().to_string()])
                .status()
                .unwrap();
            assert!(status.success());
        }
        child.wait_with_output().unwrap().into()
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
        let path = format!(
            "{}:{}",
            self.path("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
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
        let mut gh = self.gh_state();
        let failing = gh.as_object_mut().unwrap().entry("failing");
        failing
            .or_insert(json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!("api user"));
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

    /// Auto maintenance is off because a newer git detaches it into the
    /// background after a commit, where it can still be writing into
    /// `.git/objects` once the command has returned: a test script that then
    /// deletes its temporary clone with `rm -rf` fails with `Directory not
    /// empty`. `gc.auto` says the same to a git too old to know
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
