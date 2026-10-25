use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

struct Fixture {
    root: TempDir,
    home: PathBuf,
    source: PathBuf,
    bin: PathBuf,
    baseline: String,
    candidate: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        // macOS temp dirs sit behind the /var -> /private/var symlink, and the
        // command records canonical binary paths.
        let real = root.path().canonicalize().unwrap();
        let home = real.join("home");
        let source = real.join("source");
        let bin = real.join("bin");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&bin).unwrap();
        git(&source, &["init", "-q"]);
        fs::write(source.join("arm.txt"), "baseline").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "Baseline"]);
        let baseline = git(&source, &["rev-parse", "HEAD"]);
        fs::write(source.join("arm.txt"), "candidate").unwrap();
        git(&source, &["commit", "-qam", "Candidate"]);
        let candidate = git(&source, &["rev-parse", "HEAD"]);
        executable(
            &bin.join("cargo"),
            "#!/bin/sh\necho 'unexpected build' >&2\nexit 77\n",
        );
        executable(&bin.join("claude-v1"), "#!/bin/sh\necho 'claude 1.0'\n");
        symlink("claude-v1", bin.join("claude")).unwrap();
        Self {
            root,
            home,
            source,
            bin,
            baseline,
            candidate,
        }
    }

    fn cache(&self, commit: &str) -> PathBuf {
        self.home
            .join(".thirdshift/evals/builds")
            .join(commit)
            .join("thirdshift")
    }

    fn seed(&self, commit: &str) {
        let binary = self.cache(commit);
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_thirdshift"), binary).unwrap();
    }

    fn command(&self, record: &str) -> Command {
        self.command_with_harnesses(record, "claude", "claude")
    }

    fn command_with_harnesses(
        &self,
        record: &str,
        baseline_harness: &str,
        candidate_harness: &str,
    ) -> Command {
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
        command
            .args(["evals", "prepare-arms"])
            .arg(&self.source)
            .args([
                &self.baseline,
                baseline_harness,
                &self.candidate,
                candidate_harness,
            ])
            .arg(self.root.path().join(record))
            .env("HOME", &self.home)
            .env("PATH", path)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("TEST_BINARY", env!("CARGO_BIN_EXE_thirdshift"))
            .env("BUILD_LOG", self.root.path().join("builds.log"))
            .env("CARGO_TARGET_DIR", self.root.path().join("host-target"));
        command
    }

    fn prepare(&self, record: &str) -> Output {
        self.command(record).output().unwrap()
    }

    fn fake_build(&self) {
        executable(
            &self.bin.join("cargo"),
            r#"#!/bin/sh
set -eu
test "$*" = 'build --release --locked'
if git symbolic-ref -q HEAD; then
    echo 'build must use a detached worktree' >&2
    exit 78
fi
cat arm.txt >> "$BUILD_LOG"
echo >> "$BUILD_LOG"
mkdir -p "$CARGO_TARGET_DIR/release"
cp "$TEST_BINARY" "$CARGO_TARGET_DIR/release/thirdshift"
"#,
        );
    }
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn seeded_arm_commits_are_recorded_without_building() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.baseline);
    fixture.seed(&fixture.candidate);
    let arms = success(fixture.prepare("arms.json"));
    assert_eq!(arms["baseline"]["commit"], fixture.baseline);
    assert_eq!(arms["candidate"]["commit"], fixture.candidate);
    assert_eq!(
        arms["baseline"]["binary"],
        fixture.cache(&fixture.baseline).to_str().unwrap()
    );
    assert_eq!(
        arms["candidate"]["binary"],
        fixture.cache(&fixture.candidate).to_str().unwrap()
    );
    let recorded: Value =
        serde_json::from_slice(&fs::read(fixture.root.path().join("arms.json")).unwrap()).unwrap();
    assert_eq!(recorded, arms);
    success(fixture.prepare("another-experiment.json"));
}

#[test]
fn a_failed_build_keeps_its_error_and_publishes_no_arm_record() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.baseline);
    let output = fixture.prepare("arms.json");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unexpected build"), "{error}");
    assert!(error.contains(&fixture.candidate), "{error}");
    assert!(output.stdout.is_empty());
    assert!(!fixture.root.path().join("arms.json").exists());
    assert!(!fixture.cache(&fixture.candidate).exists());
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(
        !fs::read_dir(fixture.home.join(".thirdshift/evals/builds"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".arm-build-"))
    );
}

#[test]
fn cache_misses_build_each_commit_in_its_own_worktree_and_then_reuse_it() {
    let fixture = Fixture::new();
    fixture.fake_build();
    fs::write(fixture.source.join("arm.txt"), "uncommitted host change").unwrap();
    let arms = success(fixture.prepare("arms.json"));
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("builds.log")).unwrap(),
        "baseline\ncandidate\n"
    );
    for arm in ["baseline", "candidate"] {
        let output = Command::new(arms[arm]["binary"].as_str().unwrap())
            .arg("version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("thirdshift "));
    }
    success(fixture.prepare("again.json"));
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("builds.log")).unwrap(),
        "baseline\ncandidate\n"
    );
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.source.join("arm.txt")).unwrap(),
        "uncommitted host change"
    );
    assert!(!fixture.root.path().join("host-target").exists());
}

#[test]
fn concurrent_preparation_builds_each_commit_once() {
    let fixture = Fixture::new();
    fixture.fake_build();
    let mut first_command = fixture.command("first.json");
    first_command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let first = first_command.spawn().unwrap();
    let second = fixture.prepare("second.json");
    let first = first.wait_with_output().unwrap();
    success(first);
    success(second);
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("builds.log")).unwrap(),
        "baseline\ncandidate\n"
    );
}

#[test]
fn recorded_harness_path_survives_a_link_swap_between_trials() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.baseline);
    fixture.seed(&fixture.candidate);
    executable(
        &fixture.bin.join("claude-v1"),
        r#"#!/bin/sh
if [ "$1" = --version ]; then
    echo 'claude 1.0'
else
    ln -sf claude-v2 "$CLI_LINK"
    echo 'trial on claude 1.0'
fi
"#,
    );
    executable(
        &fixture.bin.join("claude-v2"),
        "#!/bin/sh\necho 'trial on claude 2.0'\n",
    );
    let arms = success(fixture.prepare("arms.json"));
    for arm in ["baseline", "candidate"] {
        assert_eq!(arms[arm]["harness"]["name"], "claude");
        assert_eq!(arms[arm]["harness"]["version"], "claude 1.0");
        assert_eq!(
            arms[arm]["harness"]["path"],
            fixture.bin.join("claude-v1").to_str().unwrap()
        );
        let trial = Command::new(arms[arm]["harness"]["path"].as_str().unwrap())
            .arg("trial")
            .env("CLI_LINK", fixture.bin.join("claude"))
            .output()
            .unwrap();
        assert!(trial.status.success());
        assert_eq!(
            String::from_utf8(trial.stdout).unwrap(),
            "trial on claude 1.0\n"
        );
    }
    let moving_link = Command::new(fixture.bin.join("claude"))
        .arg("trial")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(moving_link.stdout).unwrap(),
        "trial on claude 2.0\n"
    );
}

#[test]
fn each_distinct_harness_records_its_own_path_and_version_including_stderr() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.baseline);
    fixture.seed(&fixture.candidate);
    executable(
        &fixture.bin.join("codex-v2"),
        "#!/bin/sh\necho 'codex 2.0' >&2\n",
    );
    symlink("codex-v2", fixture.bin.join("codex")).unwrap();
    let arms = success(
        fixture
            .command_with_harnesses("arms.json", "claude", "codex")
            .output()
            .unwrap(),
    );
    assert_eq!(arms["baseline"]["harness"]["version"], "claude 1.0");
    assert_eq!(arms["candidate"]["harness"]["name"], "codex");
    assert_eq!(arms["candidate"]["harness"]["version"], "codex 2.0");
    assert_eq!(
        arms["candidate"]["harness"]["path"],
        fixture.bin.join("codex-v2").to_str().unwrap()
    );
}

#[test]
fn builds_use_the_requested_commit_even_when_git_has_a_replacement() {
    let fixture = Fixture::new();
    fixture.fake_build();
    let replacement = git(
        &fixture.source,
        &[
            "commit-tree",
            &format!("{}^{{tree}}", fixture.candidate),
            "-m",
            "Replacement",
        ],
    );
    git(
        &fixture.source,
        &["replace", &fixture.baseline, &replacement],
    );
    success(fixture.prepare("arms.json"));
    assert_eq!(
        fs::read_to_string(fixture.root.path().join("builds.log")).unwrap(),
        "baseline\ncandidate\n"
    );
}
