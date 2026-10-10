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
        crate::test_support::write_executable(
            &cli,
            &format!(
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
        );
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
    logs::begin(logs::Begin::ChildRun(
        "fixture",
        crate::logs::CommandKind::Issue,
    ));
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
        sessions.run_to_final_message(Purpose::Ordinary, "implement", &prompt)
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
fn startup_failure_never_reaches_an_installed_harness() {
    let install = tempfile::tempdir().unwrap();
    let invocation = install.path().join("invoked");
    crate::test_support::write_executable(
        &install.path().join("opencode"),
        "#!/bin/bash\nprintf 'invoked\\n' > \"$THIRDSHIFT_INSTALLED_HARNESS_INVOCATION\"\nexit 1\n",
    );
    let path = std::env::join_paths(
        std::iter::once(install.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::execution_tests::startup_failure_preserves_its_cause_without_completion_reporting",
            "--nocapture",
        ])
        .env_remove(FIXTURE)
        .env("PATH", path)
        .env("THIRDSHIFT_INSTALLED_HARNESS_INVOCATION", &invocation)
        .output()
        .unwrap();
    assert!(!invocation.exists(), "installed Harness ran: {output:?}");
    assert!(output.status.success(), "{output:?}");
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
    // A non-executable CLI doesn't stop PATH lookup. Give this child only
    // the fixture's bin, with Git for worktree setup, so no installed Harness
    // can be reached after the failed spawn.
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(git.status.success(), "{git:?}");
    let git = fs::canonicalize(String::from_utf8(git.stdout).unwrap().trim()).unwrap();
    std::os::unix::fs::symlink(git, fixture.path("bin/git")).unwrap();
    fs::set_permissions(
        fixture.path("bin/opencode"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_no_completion_reporting(
        fixture
            .command("session::execution_tests::startup_failure_preserves_its_cause_without_completion_reporting")
            .env("PATH", fixture.path("bin"))
            .output()
            .unwrap(),
    );
    assert!(!fixture.path("calls").exists());
}

// These CLIs consume the real invocation's prompt and leave opaque reports.
// No Harness-specific report transport is involved.
const LEAVE_REPORTS: &str = r#"#!/bin/bash
set -eu
if test "$(basename "$0")" = opencode; then
    prompt=$(cat)
else
    prompt=${!#}
fi
printf '%s\n' "$prompt" >> "$THIRDSHIFT_SESSION_TEST_DIR/prompts"
reports=$(printf '%s\n' "$prompt" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
test -d "$reports"
git check-ignore -q "$reports/standards.md"
printf 'Standards report\nFiles read: src/session.rs\n' > "$reports/standards.md"
printf 'Spec report\nFiles read: CONTEXT.md\n' > "$reports/spec.md"
"#;

fn review_fixture(script: &str) -> Fixture {
    let fixture = Fixture::new("");
    for harness in Harness::ALL {
        let cli = fixture.path(&format!("bin/{}", harness.name()));
        fs::write(&cli, script).unwrap();
        fs::set_permissions(cli, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fixture
}

fn with_review_sessions(
    root: &Path,
    harness: Harness,
    steps: impl FnOnce(&Sessions) -> Result<()>,
) -> (Result<()>, PathBuf) {
    logs::begin(logs::Begin::ChildRun(
        "fixture",
        crate::logs::CommandKind::Issue,
    ));
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
        harness,
        ..Choice::default()
    };
    let (result, log) = Sessions::within(&logs, &worktree, &choice, steps);
    (result, log.unwrap())
}

fn execute_review(root: &Path, harness: Harness) -> (Result<()>, PathBuf) {
    let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
    with_review_sessions(root, harness, |sessions| {
        sessions.run("implement", &prompt::fresh(&issue, "main", "issue-7"))?;
        fs::write(root.join("carried-on"), "").unwrap();
        Ok(())
    })
}

#[test]
fn keeps_review_reports_beside_each_harness_session_log() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let harness =
            Harness::named(&std::env::var("THIRDSHIFT_REVIEW_TEST_HARNESS").unwrap()).unwrap();
        let (result, log) = execute_review(root, harness);
        result.unwrap();
        assert!(log.exists());
        assert_eq!(
            fs::read_to_string(log.with_extension("standards")).unwrap(),
            "Standards report\nFiles read: src/session.rs\n"
        );
        assert_eq!(
            fs::read_to_string(log.with_extension("spec")).unwrap(),
            "Spec report\nFiles read: CONTEXT.md\n"
        );
        return;
    }
    for harness in Harness::ALL {
        let fixture = review_fixture(LEAVE_REPORTS);
        let output = fixture
            .command(
                "session::execution_tests::keeps_review_reports_beside_each_harness_session_log",
            )
            .env("THIRDSHIFT_REVIEW_TEST_HARNESS", harness.name())
            .env("XDG_DATA_HOME", fixture.path("data"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{harness:?}: {output:?}");
    }
}

#[test]
fn missing_review_reports_get_a_progress_line_and_the_run_carries_on() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let (result, log) = execute_review(root, Harness::OpenCode);
        result.unwrap();
        assert!(root.join("carried-on").exists());
        assert!(!log.with_extension("standards").exists());
        assert!(!log.with_extension("spec").exists());
        return;
    }
    let fixture = review_fixture("#!/bin/bash\ncat >/dev/null\n");
    let output = fixture
        .command("session::execution_tests::missing_review_reports_get_a_progress_line_and_the_run_carries_on")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("implement: no review reports were left; carrying on"),
        "{stderr}"
    );
}

#[test]
fn a_resumes_review_reports_are_kept_beside_its_own_session_log() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let (result, log) = execute_review(root, Harness::Claude);
        result.unwrap();
        assert_eq!(log.file_name().unwrap(), "7-fixture-implement-resume.jsonl");
        assert_eq!(
            fs::read_to_string(log.with_extension("standards")).unwrap(),
            "Resume Standards report\nFiles read: src/prompt.rs\n"
        );
        assert_eq!(
            fs::read_to_string(log.with_extension("spec")).unwrap(),
            "Resume Spec report\nFiles read: src/session.rs\n"
        );
        // The first session's reports remain with its own Session log.
        assert_eq!(
            fs::read_to_string(root.join("logs/7-fixture-implement.standards")).unwrap(),
            "Standards report\nFiles read: src/session.rs\n"
        );
        return;
    }
    let script = format!(
        r#"{LEAVE_REPORTS}
echo '{{"type":"system","subtype":"init","session_id":"s1"}}'
echo '{{"type":"result","subtype":"success"}}'
if test -f "$THIRDSHIFT_SESSION_TEST_DIR/first-session-ended"; then
    printf 'Resume Standards report\nFiles read: src/prompt.rs\n' > "$reports/standards.md"
    printf 'Resume Spec report\nFiles read: src/session.rs\n' > "$reports/spec.md"
else
    touch "$THIRDSHIFT_SESSION_TEST_DIR/first-session-ended"
    echo '{{"type":"system","subtype":"task_notification","task_id":"t1","status":"stopped","summary":"cargo test"}}'
fi
"#
    );
    let fixture = review_fixture(&script);
    let output = fixture
        .command("session::execution_tests::a_resumes_review_reports_are_kept_beside_its_own_session_log")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_failed_session_still_keeps_whichever_review_report_it_left() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let (result, log) = execute_review(root, Harness::Codex);
        assert!(result.is_err());
        assert!(!root.join("carried-on").exists());
        assert_eq!(
            fs::read_to_string(log.with_extension("standards")).unwrap(),
            "Standards report\nFiles read: src/session.rs\n"
        );
        assert!(!log.with_extension("spec").exists());
        return;
    }
    let fixture = review_fixture(&format!(
        "{LEAVE_REPORTS}\nrm \"$reports/spec.md\"\nexit 1\n"
    ));
    let output = fixture
        .command("session::execution_tests::a_failed_session_still_keeps_whichever_review_report_it_left")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn unavailable_report_setup_warns_and_the_run_carries_on() {
    if let Some(root) = std::env::var_os(FIXTURE) {
        let root = Path::new(&root);
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let (result, log) = with_review_sessions(root, Harness::OpenCode, |sessions| {
            // Skills are already linked. Only adding the report ignore pattern fails.
            let exclude = root.join("worktree/.git/info/exclude");
            let original_permissions = fs::metadata(&exclude).unwrap().permissions();
            fs::set_permissions(&exclude, fs::Permissions::from_mode(0o444)).unwrap();
            let result = sessions.run("implement", &prompt::fresh(&issue, "main", "issue-7"));
            fs::set_permissions(&exclude, original_permissions).unwrap();
            result?;
            fs::write(root.join("carried-on"), "").unwrap();
            Ok(())
        });
        result.unwrap();
        assert!(log.exists());
        assert!(root.join("carried-on").exists());
        let received = fs::read_to_string(root.join("prompts")).unwrap();
        assert!(!received.contains(prompt::REVIEW_REPORTS_DIRECTORY));
        assert!(!received.contains("Write the reviewers' reports"));
        assert!(received.contains("run its test or command as written"));
        return;
    }
    let fixture = review_fixture("#!/bin/bash\ncat > \"$THIRDSHIFT_SESSION_TEST_DIR/prompts\"\n");
    let output = fixture
        .command("session::execution_tests::unavailable_report_setup_warns_and_the_run_carries_on")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not prepare review reports"),
        "{stderr}"
    );
}
