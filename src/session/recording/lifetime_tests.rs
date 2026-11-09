//! Real detached work, with cleanup restricted to the fixture's recorded PID.

use super::*;

struct Detached(tempfile::TempDir);

impl Detached {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn pid(&self) -> libc::pid_t {
        fs::read_to_string(self.0.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn running(&self) -> bool {
        let output = Command::new("ps")
            .args(["-p", &self.pid().to_string(), "-o", "stat="])
            .output()
            .unwrap();
        let state = String::from_utf8_lossy(&output.stdout);
        !state.trim().is_empty() && !state.trim().starts_with('Z')
    }

    fn run(&self, tail: &str, stopped: bool) -> Result<Ended> {
        self.run_script(&format!("{DOUBLE_FORK}\n{tail}"), stopped)
    }

    fn run_script(&self, script: &str, stopped: bool) -> Result<Ended> {
        let mut launched = crate::scope::session_command("python3");
        launched
            .command()
            .args(["-c", script])
            .current_dir(self.0.path());
        Recording::for_trial().run(
            "implement",
            &Choice::default(),
            launched.as_mut(),
            Input {
                deadline: Some(Instant::now() + Duration::from_secs(if stopped { 1 } else { 15 })),
                ..Input::default()
            },
            &self.0.path().join("session.jsonl"),
        )
    }
}

impl Drop for Detached {
    fn drop(&mut self) {
        if self.0.path().join("pid").exists() {
            let _ = crate::process::signal(self.pid(), libc::SIGKILL);
        }
    }
}

#[test]
fn a_normally_ended_session_kills_double_forked_work() {
    let detached = Detached::new();
    detached.run(SUCCESS, false).unwrap();
    assert!(
        !detached.running(),
        "double-forked sleep survived a normally ended Harness session"
    );
}

const SUCCESS: &str = r#"print('{"type":"result","subtype":"success","result":"Done"}')"#;
const DOUBLE_FORK: &str = r#"
import os, time
from pathlib import Path
root = Path.cwd()
child = os.fork()
if child == 0:
    os.setsid()
    middle = os.getpid()
    if os.fork() != 0:
        os._exit(0)
    for fd in range(3):
        target = os.open('/dev/null', os.O_RDWR)
        os.dup2(target, fd)
        if target > 2:
            os.close(target)
    while os.getppid() == middle:
        time.sleep(0.001)
    (root / 'pid').write_text(str(os.getpid()))
    (root / 'marker').write_text(os.environ.get('THIRDSHIFT_SESSION', ''))
    os.execlp('sleep', 'sleep', '600')
os.waitpid(child, 0)
deadline = time.monotonic() + 5
while not (root / 'pid').exists():
    assert time.monotonic() < deadline, 'detached child did not become ready'
    time.sleep(0.001)
"#;

#[test]
fn a_stopped_session_kills_double_forked_work() {
    let detached = Detached::new();
    let error = detached.run("time.sleep(600)", true).unwrap_err();
    assert!(error.is::<crate::process::TimedOut>(), "{error:#}");
    assert!(
        !detached.running(),
        "double-forked sleep survived the stop path"
    );
}

#[test]
fn a_failed_session_kills_double_forked_work() {
    let detached = Detached::new();
    assert!(detached.run("raise SystemExit(23)", false).is_err());
    assert!(
        !detached.running(),
        "double-forked sleep survived a failed session"
    );
}

#[test]
fn unavailable_user_manager_uses_the_marker_sweep() {
    const CHILD: &str = "THIRDSHIFT_LIFETIME_NO_MANAGER";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::recording::lifetime_tests::unavailable_user_manager_uses_the_marker_sweep", "--nocapture"])
            .env(CHILD, "1")
            .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}/missing-bus", root.path().display()))
            .env("XDG_RUNTIME_DIR", root.path())
            .output().unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }
    assert!(!crate::scope::available());
    a_normally_ended_session_kills_double_forked_work();
    a_stopped_session_kills_double_forked_work();
    sweeping_one_session_leaves_another_live_session_alone();
}

#[test]
fn sweep_failures_warn_without_changing_the_session_outcome() {
    const CHILD: &str = "THIRDSHIFT_LIFETIME_SWEEP_FAILURE";
    if std::env::var_os(CHILD).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::recording::lifetime_tests::sweep_failures_warn_without_changing_the_session_outcome", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.matches("injected session-sweep failure").count(),
            2,
            "{stderr}"
        );
        assert!(
            stderr.contains("warning: could not fully sweep session"),
            "{stderr}"
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    for status in [0, 23] {
        let script = format!("{SUCCESS}\nraise SystemExit({status})");
        let result = Recording::for_trial().run(
            "implement",
            &Choice::default(),
            Command::new("python3")
                .args(["-c", &script])
                .env("THIRDSHIFT_EXECUTION_TEST_FAULT", "session-sweep")
                .current_dir(root.path()),
            Input::default(),
            &root.path().join("session.jsonl"),
        );
        if status == 0 {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(
                !format!("{error:#}").contains("injected session-sweep"),
                "{error:#}"
            );
        }
    }
}

#[test]
fn version_detection_during_a_live_session_preserves_its_work() {
    let root = tempfile::tempdir().unwrap();
    let program = root.path().join("harness");
    crate::test_support::write_executable(
        &program,
        r#"#!/bin/sh
if [ "$1" = --version ]; then echo fixture-harness-1.0; exit 0; fi
echo '{"type":"assistant","message":{"model":"fixture","content":[]}}'
sleep 0.5
echo '{"type":"result","subtype":"success","result":"Done"}'
touch completed
"#,
    );
    let mut launched = crate::scope::session_command(&program);
    launched.command().current_dir(root.path());
    let log = root.path().join("session.jsonl");
    Recording::for_trial()
        .run(
            "implement",
            &Choice::default(),
            launched.as_mut(),
            Input::default(),
            &log,
        )
        .unwrap();
    assert!(
        root.path().join("completed").exists(),
        "version probe killed its live session"
    );
    let trace = fs::read_to_string(log.with_extension("logs.jsonl")).unwrap();
    assert!(
        trace.contains("fixture-harness-1.0"),
        "scope wrapper replaced the Harness version: {trace}"
    );
}

#[test]
fn sweeping_one_session_leaves_another_live_session_alone() {
    let first = Detached::new();
    let second = Detached::new();
    std::thread::scope(|threads| {
        let active = threads.spawn(|| {
            second.run(
                r#"
deadline = time.monotonic() + 10
while not (root / 'release').exists():
    assert time.monotonic() < deadline, 'parallel fixture was never released'
    time.sleep(0.01)
"#,
                false,
            )
        });
        struct Release<'a>(&'a Path);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                let _ = fs::write(self.0.join("release"), "");
            }
        }
        let release = Release(second.0.path());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !second.0.path().join("marker").exists() {
            assert!(
                Instant::now() < deadline,
                "second session never became ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        first.run(SUCCESS, false).unwrap();
        assert!(!first.running());
        assert!(
            second.running(),
            "a different live session's work was killed"
        );
        assert_ne!(
            fs::read(first.0.path().join("marker")).unwrap(),
            fs::read(second.0.path().join("marker")).unwrap()
        );
        drop(release);
        active.join().unwrap().unwrap();
    });
    assert!(!second.running());
}

#[test]
fn a_live_scope_catches_work_that_clears_its_environment() {
    if !crate::scope::available() {
        eprintln!("skipped: requires a systemd user manager");
        return;
    }
    let script = DOUBLE_FORK
        .replace(
            "(root / 'pid').write_text(str(os.getpid()))",
            "(root / 'before-exec').write_text(str(os.getpid()))",
        )
        .replace(
            "os.execlp('sleep', 'sleep', '600')",
            "os.execve('/bin/sh', ['sh', '-c', 'echo $$ > pid; exec sleep 600'], {})",
        );
    for stopped in [false, true] {
        let detached = Detached::new();
        let tail = if stopped { "time.sleep(600)" } else { SUCCESS };
        let result = detached.run_script(&format!("{script}\n{tail}"), stopped);
        if stopped {
            assert!(result.unwrap_err().is::<crate::process::TimedOut>());
        } else {
            result.unwrap();
        }
        assert!(
            !detached.running(),
            "scope missed detached work with no environment marker (stopped: {stopped})"
        );
    }
}

#[test]
fn detached_work_ignoring_sigterm_is_killed_after_the_grace_period() {
    let detached = Detached::new();
    let script = DOUBLE_FORK
        .replace("import os, time", "import os, time, signal")
        .replace(
            "os.execlp('sleep', 'sleep', '600')",
            "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n    os.execlp('sleep', 'sleep', '600')",
        );
    detached
        .run_script(&format!("{script}\n{SUCCESS}"), false)
        .unwrap();
    assert!(
        !detached.running(),
        "SIGTERM-ignoring detached work survived SIGKILL escalation"
    );
}
