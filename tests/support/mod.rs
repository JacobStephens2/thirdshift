//! Test harness: runs the compiled `thirdshift` binary against real git and
//! fake `gh` and `claude` executables.
//!
//! Layout of a scenario's temp root:
//!
//! ```text
//! origin.git/        bare repo standing in for github.com/<owner>/<repo>;
//!                    it rejects non-fast-forward pushes, so no rebase or
//!                    force-push can reach it
//! home/              $HOME: .gitconfig with identity and the insteadOf rule
//! bin/               fake gh and claude, first on PATH
//! tmp/               $TMPDIR, so leftover temp directories are visible
//! work/<repo>/       the launch clone, origin https://github.com/<owner>/<repo>.git
//! gh-state.json      fake GitHub state
//! claude-script.sh   what the fake agent does this test
//! claude-script.sh.<n>  what it does in the n-th session instead, if present
//! claude-calls.json  what the fake agent was asked to do
//! gh-calls.json      every gh command run, by thirdshift or the fake agent
//! ```

#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

pub const OWNER: &str = "acme";
pub const REPO: &str = "widgets";

pub struct Scenario {
    root: TempDir,
    /// `root`'s path with symlinks resolved, as git reports it: on macOS the
    /// temp directory is under `/var`, a symlink to `/private/var`.
    dir: PathBuf,
}

pub struct RunResult {
    pub stdout: String,
    pub stderr: String,
    pub code: Option<i32>,
}

impl From<Output> for RunResult {
    fn from(output: Output) -> Self {
        RunResult {
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
            code: output.status.code(),
        }
    }
}

impl Scenario {
    /// An origin with one commit on `main`, a launch clone of it with `main`
    /// checked out, and an open issue #7.
    pub fn new() -> Self {
        let root = TempDir::new().unwrap();
        let dir = root.path().canonicalize().unwrap();
        let scenario = Scenario { root, dir };
        for dir in ["home", "bin", "tmp", "work"] {
            fs::create_dir_all(scenario.path(dir)).unwrap();
        }
        scenario.write_gitconfig();
        scenario.install_fakes();
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
        self.dir.join(relative)
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

    pub fn run(&self, args: &[&str]) -> RunResult {
        self.command(args).output().unwrap().into()
    }

    /// Run thirdshift and send it `signal` (e.g. `"INT"`) once the fake agent
    /// has touched the file `started` in the scenario root.
    pub fn run_and_signal(&self, args: &[&str], started: &str, signal: &str) -> RunResult {
        let child = self
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.path(started).exists() {
            assert!(Instant::now() < deadline, "the agent never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        let status = Command::new("kill")
            .args([&format!("-{signal}"), &child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        child.wait_with_output().unwrap().into()
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
            .env("TMPDIR", self.path("tmp"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("FAKE_GH_STATE", self.path("gh-state.json"))
            .env("FAKE_CLAUDE_SCRIPT", self.path("claude-script.sh"))
            .env("FAKE_CLAUDE_RECORD", self.path("claude-calls.json"))
            .env("FAKE_GH_RECORD", self.path("gh-calls.json"))
            // Seconds of waiting for CI become milliseconds.
            .env("THIRDSHIFT_CI_GRACE_MS", "300")
            .env("THIRDSHIFT_POLL_MS", "10");
        command
    }

    /// Set issue `number`'s state on the fake GitHub: `"OPEN"` or `"CLOSED"`.
    pub fn issue_is(&self, number: u32, state: &str) {
        let mut gh = self.gh_state();
        gh["issues"][number.to_string()] = json!(state);
        self.write_gh_state(&gh);
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

    /// Push `branch` to origin: `from` plus one commit per subject in
    /// `commits`, oldest first.
    pub fn origin_has_branch(&self, branch: &str, from: &str, commits: &[&str]) {
        let seed = self.path("seed");
        git(&self.path(""), &["clone", "-q", &self.github_url(), "seed"]);
        git(
            &seed,
            &["checkout", "-q", "-b", branch, &format!("origin/{from}")],
        );
        for (i, subject) in commits.iter().enumerate() {
            fs::write(seed.join(format!("{branch}-{i}.txt")), subject).unwrap();
            git(&seed, &["add", "."]);
            git(&seed, &["commit", "-q", "-m", subject]);
        }
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

    fn write_gitconfig(&self) {
        let config = format!(
            "[user]\n\tname = Test Runner\n\temail = runner@example.com\n\
             [init]\n\tdefaultBranch = main\n\
             [url \"{origin}\"]\n\tinsteadOf = {github}\n",
            origin = self.origin_dir().display(),
            github = self.github_url(),
        );
        fs::write(self.path("home/.gitconfig"), config).unwrap();
    }

    fn install_fakes(&self) {
        let fakes = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fakes");
        for name in ["gh", "claude"] {
            let target = self.path("bin").join(name);
            fs::copy(fakes.join(name), &target).unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        }
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
