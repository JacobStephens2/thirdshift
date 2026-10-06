//! Observable execution behavior with finite, local child Run substitutes.

use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;
use tempfile::TempDir;

const PR: &str = "https://github.com/acme/widgets/pull/31";

/// Private fault points retain a real child and the normal execution path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Fault {
    StdoutPipe,
    StderrPipe,
    StdoutStart,
    StderrStart,
    StdoutRead,
    StderrRead,
    StdoutPanic,
    StderrPanic,
    Wait,
}

impl Fault {
    pub(super) fn fire(self) -> Result<()> {
        if matches!(self, Self::StdoutPanic | Self::StderrPanic) {
            panic!("injected {self:?}");
        }
        bail!("injected {self:?}");
    }
}

struct Fixture(TempDir);

impl Fixture {
    fn new(script: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("run");
        fs::write(
            &executable,
            format!(
                "#!/usr/bin/env python3\nimport os, signal, time\nsignal.alarm(8)\nROOT = {:?}\nwith open(ROOT + '/pid', 'w') as f: f.write(str(os.getpid()))\n{}\n",
                dir.path(), script
            ),
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        Self(dir)
    }

    fn start(&self) -> Result<Handle> {
        self.start_using(Handle::start_readers)
    }

    fn start_using(&self, startup: impl FnOnce(&mut Handle) -> Result<()>) -> Result<Handle> {
        super::start_using(
            &self.0.path().join("run"),
            &IssueUrl::parse("https://github.com/acme/widgets/issues/248").unwrap(),
            &Given {
                kind: Kind::Ticket {
                    spec_branch: "issue-237".to_string(),
                },
                stamp: "20261003T120000-0400".to_string(),
                base_fix: BaseFixAsk::Forbid,
                harness: Choice::default(),
            },
            startup,
        )
    }

    fn await_file(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !self.0.path().join(name).exists() {
            assert!(Instant::now() < deadline, "fixture never wrote {name}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.path().join(name)).unwrap()
    }

    fn assert_stopped(&self) {
        let pid = self.read("pid").parse::<libc::pid_t>().unwrap();
        // SAFETY: probe only the exact PID recorded by this finite fixture.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "child {pid} survived");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(self.0.path().join("pid"))
            && let Ok(pid) = pid.parse::<libc::pid_t>()
        {
            // SAFETY: assertion-failure cleanup uses this fixture's exact PID.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// Global interruption is exercised only in an isolated test process.
fn isolated(name: &str) -> Option<std::process::Output> {
    if std::env::var("THIRDSHIFT_TEST_CHILD_RUN").as_deref() == Ok(name) {
        return None;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("child_run::execution_tests::{name}"),
            "--nocapture",
        ])
        .env("THIRDSHIFT_TEST_CHILD_RUN", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(output)
}

#[test]
fn malformed_stderr_preserves_a_successful_ending_and_base_fix_account() {
    if let Some(output) =
        isolated("malformed_stderr_preserves_a_successful_ending_and_base_fix_account")
    {
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("#248: �diagnostic\n"), "{stderr}");
        assert!(!stderr.contains('\r'), "{stderr}");
        return;
    }
    let fixture = Fixture::new(&format!(
        "os.write(2, b'\\xffdiagnostic\\nthirdshift: Base fix: https://github.com/acme/widgets/issues/8 merged\\r\\n')\nos.write(1, b'earlier output\\n{PR}\\n')"
    ));

    let ended = fixture.start().unwrap().wait().unwrap();

    match ended {
        Ended::Reached { pr_url, base_fix } => {
            assert_eq!(pr_url.as_deref(), Some(PR));
            assert_eq!(
                base_fix.as_deref(),
                Some("https://github.com/acme/widgets/issues/8 merged")
            );
        }
        _ => panic!("the successful child lost its ending"),
    }
}

#[test]
fn stdout_pipe_pressure_completes_with_the_last_line_as_the_pr() {
    let fixture = Fixture::new(&format!(
        "os.write(1, b'x' * (1024 * 1024))\nos.write(2, b'pressure drained\\n')\nos.write(1, b'\\n{PR}\\n')\nopen(ROOT + '/drained', 'w').close()"
    ));
    let started = Instant::now();

    let handle = fixture.start().unwrap();
    fixture.await_file("drained");
    let ended = handle.wait().unwrap();

    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(matches!(ended, Ended::Reached { pr_url: Some(url), .. } if url == PR));
    fixture.assert_stopped();
}

#[test]
fn malformed_diagnostics_preserve_later_failure_cause_and_final_session_log() {
    let fixture = Fixture::new(
        "os.write(2, b'bad \\xff\\nthirdshift: inherited failure\\r\\nthirdshift: #9: descendant failure\\nthirdshift: session log: /logs/248.jsonl')\nraise SystemExit(7)",
    );
    let ended = fixture.start().unwrap().wait().unwrap();
    assert!(matches!(ended, Ended::Failed { cause, log: Some(log) }
        if cause == "inherited failure" && log == "/logs/248.jsonl"));
    fixture.assert_stopped();
}

#[test]
fn polling_is_pending_until_the_child_finishes_and_delivers_its_ending_once() {
    let fixture = Fixture::new(&format!(
        "open(ROOT + '/ready', 'w').close()\nwhile not os.path.exists(ROOT + '/finish'): time.sleep(.01)\nos.write(1, b'{PR}\\n')"
    ));
    let mut handle = fixture.start().unwrap();
    fixture.await_file("ready");
    let started = Instant::now();
    assert!(handle.try_wait().is_none());
    assert!(started.elapsed() < Duration::from_millis(50));
    fs::write(fixture.0.path().join("finish"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    let ended = loop {
        if let Some(ended) = handle.try_wait() {
            break ended.unwrap();
        }
        assert!(Instant::now() < deadline, "polling never completed");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(matches!(ended, Ended::Reached { pr_url: Some(url), .. } if url == PR));
    assert!(handle.try_wait().is_none());
    fixture.assert_stopped();
}

const SAVES_ON_STOP: &str = r#"
def stop(signum, frame):
    with open(ROOT + '/terms', 'a') as f: f.write('TERM\n')
    os.write(1, b'x' * (1024 * 1024))
    os.write(2, b'thirdshift: interrupted\n')
    open(ROOT + '/saved', 'w').close()
    raise SystemExit(1)
signal.signal(signal.SIGTERM, stop)
open(ROOT + '/ready', 'w').close()
while True: time.sleep(.01)
"#;

#[test]
fn dropping_a_live_handle_gracefully_saves_work_and_reaps_the_child() {
    let fixture = Fixture::new(SAVES_ON_STOP);
    let handle = fixture.start().unwrap();
    fixture.await_file("ready");
    drop(handle);
    assert!(fixture.0.path().join("saved").exists());
    assert_eq!(fixture.read("terms"), "TERM\n");
    fixture.assert_stopped();
}

#[test]
fn invalid_stdout_stops_a_live_child_and_preserves_the_numbered_transport_cause() {
    let fixture = Fixture::new(
        "def stop(signum, frame):\n    open(ROOT + '/saved', 'w').close()\n    raise SystemExit(1)\nsignal.signal(signal.SIGTERM, stop)\nos.write(1, b'\\xff')\nos.close(1)\nwhile True: time.sleep(.01)",
    );
    let started = Instant::now();
    let error = fixture.start().unwrap().wait().unwrap_err();
    let cause = format!("{error:#}");
    assert!(
        cause.starts_with("could not read the Run for #248: "),
        "{cause}"
    );
    assert!(cause.contains("valid UTF-8"), "{cause}");
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(fixture.0.path().join("saved").exists());
    fixture.assert_stopped();
}

#[test]
fn every_post_spawn_startup_failure_stops_and_reaps_its_child() {
    for fault in [
        Fault::StdoutPipe,
        Fault::StdoutStart,
        Fault::StderrPipe,
        Fault::StderrStart,
    ] {
        let fixture = Fixture::new(
            r#"
def stop(signum, frame):
    open(ROOT + '/saved', 'w').close()
    raise SystemExit(1)
signal.signal(signal.SIGTERM, stop)
open(ROOT + '/ready', 'w').close()
while True: time.sleep(.01)
"#,
        );
        let result = fixture.start_using(|owned| {
            fixture.await_file("ready");
            owned.fault = Some(fault);
            owned.start_readers()
        });
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("{fault:?} did not fail startup"),
        };
        let cause = format!("{error:#}");
        assert!(cause.contains("#248"), "{cause}");
        assert!(cause.contains(&format!("injected {fault:?}")), "{cause}");
        assert!(fixture.0.path().join("saved").exists(), "{fault:?}");
        fixture.assert_stopped();
    }
}

const QUIET_SAVES_ON_STOP: &str = r#"
def stop(signum, frame):
    open(ROOT + '/saved', 'w').close()
    os.write(2, b'thirdshift: cleanup failed\n')
    raise SystemExit(1)
signal.signal(signal.SIGTERM, stop)
open(ROOT + '/ready', 'w').close()
while True: time.sleep(.01)
"#;

#[test]
fn reader_failures_panics_and_wait_failure_keep_their_first_cause_after_cleanup() {
    for fault in [
        Fault::StdoutRead,
        Fault::StderrRead,
        Fault::StdoutPanic,
        Fault::StderrPanic,
        Fault::Wait,
    ] {
        let fixture = Fixture::new(QUIET_SAVES_ON_STOP);
        let handle = fixture
            .start_using(|owned| {
                fixture.await_file("ready");
                owned.fault = Some(fault);
                owned.start_readers()
            })
            .unwrap();
        let started = Instant::now();
        let cause = format!("{:#}", handle.wait().unwrap_err());
        assert!(cause.contains("#248"), "{fault:?}: {cause}");
        if matches!(fault, Fault::StdoutPanic | Fault::StderrPanic) {
            assert!(cause.contains("panicked"), "{cause}");
        }
        assert!(cause.contains(&format!("injected {fault:?}")), "{cause}");
        assert!(!cause.contains("cleanup failed"), "{cause}");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(fixture.0.path().join("saved").exists(), "{fault:?}");
        fixture.assert_stopped();
    }
}

const GATED_STOP: &str = r#"
def stop(signum, frame):
    with open(ROOT + '/terms', 'a') as f: f.write('TERM\n')
    open(ROOT + '/stopping', 'w').close()
    while not os.path.exists(ROOT + '/finish'): time.sleep(.01)
    open(ROOT + '/saved', 'w').close()
    raise SystemExit(0)
signal.signal(signal.SIGTERM, stop)
open(ROOT + '/ready', 'w').close()
while True: time.sleep(.01)
"#;

fn poll_until_stopping(handle: &mut Handle, fixture: &Fixture) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !fixture.0.path().join("stopping").exists() {
        assert!(
            handle.try_wait().is_none(),
            "a stopping child reported completion early"
        );
        assert!(
            Instant::now() < deadline,
            "the child never started stopping"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let started = Instant::now();
    assert!(handle.try_wait().is_none());
    assert!(started.elapsed() < Duration::from_millis(50));
}

#[test]
fn a_transport_failure_stays_pending_while_the_child_saves_work() {
    let fixture = Fixture::new(GATED_STOP);
    let mut handle = fixture
        .start_using(|owned| {
            fixture.await_file("ready");
            owned.fault = Some(Fault::StderrRead);
            owned.start_readers()
        })
        .unwrap();
    poll_until_stopping(&mut handle, &fixture);
    fs::write(fixture.0.path().join("finish"), "").unwrap();
    let cause = format!("{:#}", handle.wait().unwrap_err());
    assert_eq!(
        cause,
        "could not read the Run for #248: injected StderrRead"
    );
    assert_eq!(fixture.read("terms"), "TERM\n");
    assert!(fixture.0.path().join("saved").exists());
    fixture.assert_stopped();
}

#[test]
fn interruption_is_forwarded_once_and_waits_for_successful_graceful_cleanup() {
    if isolated("interruption_is_forwarded_once_and_waits_for_successful_graceful_cleanup")
        .is_some()
    {
        return;
    }
    interrupt::install().unwrap();
    let fixture = Fixture::new(GATED_STOP);
    let mut handle = fixture.start().unwrap();
    fixture.await_file("ready");
    signal_hook::low_level::raise(libc::SIGINT).unwrap();
    poll_until_stopping(&mut handle, &fixture);
    signal_hook::low_level::raise(libc::SIGTERM).unwrap();
    assert!(handle.try_wait().is_none());
    fs::write(fixture.0.path().join("finish"), "").unwrap();
    assert!(matches!(handle.wait().unwrap(), Ended::Interrupted));
    assert_eq!(fixture.read("terms"), "TERM\n");
    assert!(fixture.0.path().join("saved").exists());
    fixture.assert_stopped();
}

#[test]
fn interruption_during_transport_cleanup_takes_precedence() {
    if isolated("interruption_during_transport_cleanup_takes_precedence").is_some() {
        return;
    }
    interrupt::install().unwrap();
    let fixture = Fixture::new(GATED_STOP);
    let mut handle = fixture
        .start_using(|owned| {
            fixture.await_file("ready");
            owned.fault = Some(Fault::StdoutRead);
            owned.start_readers()
        })
        .unwrap();
    poll_until_stopping(&mut handle, &fixture);
    signal_hook::low_level::raise(libc::SIGTERM).unwrap();
    assert!(handle.try_wait().is_none());
    fs::write(fixture.0.path().join("finish"), "").unwrap();
    assert!(matches!(handle.wait().unwrap(), Ended::Interrupted));
    assert_eq!(fixture.read("terms"), "TERM\n");
    fixture.assert_stopped();
}
