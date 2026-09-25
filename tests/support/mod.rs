//! Test harness: runs the compiled `thirdshift` binary against real git and
//! fake `gh` and `claude` executables.
//!
//! Layout of a scenario's temp root:
//!
//! ```text
//! origin.git/        bare repo standing in for github.com/<owner>/<repo>
//! home/              $HOME: .gitconfig with identity and the insteadOf rule
//! bin/               fake gh and claude, first on PATH
//! tmp/               $TMPDIR, so leftover temp directories are visible
//! work/<repo>/       the launch clone, origin https://github.com/<owner>/<repo>.git
//! gh-state.json      fake GitHub state
//! claude-script.sh   what the fake agent does this test
//! claude-calls.json  what the fake agent was asked to do
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
    /// An origin with one commit on `main`, and a launch clone of it with
    /// `main` checked out.
    pub fn new() -> Self {
        let root = TempDir::new().unwrap();
        let scenario = Scenario { root };
        for dir in ["home", "bin", "tmp", "work"] {
            fs::create_dir_all(scenario.path(dir)).unwrap();
        }
        scenario.write_gitconfig();
        scenario.install_fakes();
        scenario.write_gh_state(&json!({ "repo": format!("{OWNER}/{REPO}"), "prs": [] }));
        scenario.agent_does("true");

        git(
            &scenario.path(""),
            &["init", "--bare", "--initial-branch=main", "origin.git"],
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
        self.root.path().join(relative)
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
            .env("FAKE_CLAUDE_RECORD", self.path("claude-calls.json"));
        command
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

    /// Output of a git command in the origin repo, panicking on failure.
    pub fn origin_git(&self, args: &[&str]) -> String {
        git(&self.origin_dir(), args)
    }

    /// Create `branch` on origin, pointing at `main`.
    pub fn origin_has_branch(&self, branch: &str) {
        self.origin_git(&["branch", branch, "main"]);
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

    /// Names of the files and directories directly inside `relative`.
    pub fn entries(&self, relative: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.path(relative))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
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
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap())
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
