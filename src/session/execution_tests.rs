//! Exercise real Session execution with local CLIs in isolated processes.

use super::*;
use crate::harness::Harness;
use crate::interrupt;
use std::os::unix::fs::PermissionsExt;
use std::process::{Output, Stdio};

const FIXTURE: &str = "THIRDSHIFT_SESSION_TEST_DIR";
const WARNING: &str = "warning: the session never loaded thirdshift-implement with its skill tool";
const USAGE: &str = "42 input tokens (0 cache read, 0 cache write), 7 output tokens (0 reasoning)";

struct Fixture(tempfile::TempDir);

impl Fixture {
    fn new(script: &str) -> Self {
        let fixture = Self(tempfile::tempdir().unwrap());
        fs::create_dir(fixture.path("bin")).unwrap();
        fs::create_dir(fixture.path("logs")).unwrap();
        let cli = fixture.path("bin/opencode");
        fs::write(
            &cli,
            format!(
                r#"#!/bin/bash
if test "$1" = session; then
    echo export >> "$THIRDSHIFT_SESSION_TEST_DIR/calls"
    cat "$THIRDSHIFT_SESSION_TEST_DIR/export.json"
    exit 0
fi
echo session >> "$THIRDSHIFT_SESSION_TEST_DIR/calls"
echo $$ > "$THIRDSHIFT_SESSION_TEST_DIR/pid"
echo '{{"type":"step_start","sessionID":"s1","part":{{}}}}'
{script}
"#
            ),
        )
        .unwrap();
        fs::set_permissions(cli, fs::Permissions::from_mode(0o755)).unwrap();
        // Retained failure and usage must not replace the transport error.
        fs::write(
            fixture.path("export.json"),
            r#"{"info":{"outcome":"failed","error":{"message":"later retained failure"}},"messages":[{"type":"assistant","tokens":{"input":42,"output":7},"content":[{"type":"text","text":"retained reply"}]}]}"#,
        )
        .unwrap();
        fixture
    }

    fn path(&self, path: &str) -> PathBuf {
        self.0.path().join(path)
    }

    fn command(&self, test: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let path = std::env::join_paths(
            std::iter::once(self.path("bin"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        command
            .args(["--exact", test, "--nocapture"])
            .env(FIXTURE, self.0.path())
            .env("PATH", path);
        command
    }

    fn assert_reporting(&self, output: Output) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(stderr.matches(WARNING).count(), 1, "{stderr}");
        assert_eq!(stderr.matches(USAGE).count(), 1, "{stderr}");
        assert_eq!(stderr.matches("session ended after").count(), 1, "{stderr}");
        assert!(!stderr.contains("resuming"), "{stderr}");
        assert_eq!(
            fs::read_to_string(self.path("calls")).unwrap(),
            "session\nexport\n"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(self.path("pid"))
            && let Ok(pid) = pid.trim().parse::<libc::pid_t>()
        {
            // SAFETY: clean up only this fixture's exact child and group if
            // an assertion failed before production cleanup could finish.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

fn execute_session(root: &Path) -> anyhow::Error {
    interrupt::install().unwrap();
    logs::begin(logs::Begin::ChildRun("fixture"));
    let worktree = root.join("worktree");
    fs::create_dir(&worktree).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&worktree)
            .status()
            .unwrap()
            .success()
    );
    let logs = Logs {
        name: "7".into(),
        dir: root.join("logs"),
    };
    let choice = Choice {
        harness: Harness::OpenCode,
        ..Choice::default()
    };
    let prompt = format!("/thirdshift-implement {}", "p".repeat(1024 * 1024));
    let (result, log) = Sessions::within(&logs, &worktree, &choice, |sessions| {
        sessions.run_to_final_message("implement", &prompt)
    });
    assert!(log.unwrap().exists());
    result.unwrap_err()
}

fn assert_owned_child_stopped(root: &Path) {
    let pid: libc::pid_t = fs::read_to_string(root.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: probe only this fixture's recorded PID.
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "owned child survived");
}

#[test]
fn completed_stdout_is_reported_after_a_broken_prompt() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let error = execute_session(root);
        assert!(
            error
                .to_string()
                .contains("could not write the prompt to opencode"),
            "{error:#}"
        );
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert_owned_child_stopped(root);
        return;
    }
    let fixture = Fixture::new(
        r#"echo '{"type":"error","sessionID":"s1","error":{"message":"later turn failure"}}'
exec 1>&-
for attempt in {1..500}; do
    grep -q 'later turn failure' "$THIRDSHIFT_SESSION_TEST_DIR"/logs/*.jsonl && break
    sleep 0.01
done
exec 0<&-
exec sleep 5"#,
    );
    fixture.assert_reporting(
        fixture
            .command("session::execution_tests::completed_stdout_is_reported_after_a_broken_prompt")
            .output()
            .unwrap(),
    );
}

#[test]
fn partial_stdout_is_reported_after_log_failure_with_a_blocked_prompt() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let error = execute_session(root);
        assert!(
            error.to_string().contains("could not write")
                && error.to_string().contains("implement.jsonl"),
            "{error:#}"
        );
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert_owned_child_stopped(root);
        return;
    }
    let fixture = Fixture::new(
        r#"echo ready-to-fail
for attempt in {1..500}; do
    test -f "$THIRDSHIFT_SESSION_TEST_DIR/log-closed" && break
    sleep 0.01
done
echo another-line
exec sleep 5"#,
    );
    let log = fixture.path("logs/7-fixture-implement.jsonl");
    assert!(Command::new("mkfifo").arg(&log).status().unwrap().success());
    let child = fixture
        .command("session::execution_tests::partial_stdout_is_reported_after_log_failure_with_a_blocked_prompt")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(File::open(log).unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 || line == "ready-to-fail\n" {
            break;
        }
    }
    drop(reader);
    fs::write(fixture.path("log-closed"), "").unwrap();
    fixture.assert_reporting(child.wait_with_output().unwrap());
}

fn assert_no_completion_reporting(output: Output) {
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for unexpected in [WARNING, USAGE, "session ended after", "resuming"] {
        assert!(!stderr.contains(unexpected), "{stderr}");
    }
}

#[test]
fn interruption_suppresses_recovered_interpretation_and_retained_reads() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let error = execute_session(root);
        assert_eq!(format!("{error:#}"), "interrupted");
        assert_owned_child_stopped(root);
        return;
    }
    let fixture = Fixture::new(
        r#"exec 1>&-
for attempt in {1..500}; do
    grep -q 'sessionID' "$THIRDSHIFT_SESSION_TEST_DIR"/logs/*.jsonl && break
    sleep 0.01
done
kill -INT "$PPID"
exec sleep 5"#,
    );
    assert_no_completion_reporting(
        fixture.command("session::execution_tests::interruption_suppresses_recovered_interpretation_and_retained_reads").output().unwrap(),
    );
    assert_eq!(
        fs::read_to_string(fixture.path("calls")).unwrap(),
        "session\n"
    );
}

#[test]
fn startup_failure_preserves_its_cause_without_completion_reporting() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let error = execute_session(Path::new(&root));
        assert_eq!(error.to_string(), "could not run opencode");
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        return;
    }
    let fixture = Fixture::new("exit 0");
    // Leave the selected CLI on PATH, but make spawning it fail. This never
    // falls through to a real installed Harness.
    fs::set_permissions(
        fixture.path("bin/opencode"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_no_completion_reporting(
        fixture.command("session::execution_tests::startup_failure_preserves_its_cause_without_completion_reporting").output().unwrap(),
    );
    assert!(!fixture.path("calls").exists());
}
