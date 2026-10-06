//! Behavior through the same two operations used by sessions and checks.

use super::*;
use crate::harness::Harness;

use std::fs;
use std::path::Path;
use std::time::Instant;

use tempfile::TempDir;

fn shell(script: &str) -> Command {
    let mut command = Command::new("bash");
    command.args(["-c", script]);
    command
}

#[test]
fn captured_completion_preserves_both_byte_streams_and_nonzero_status() {
    let result = output(
        Harness::OpenCode.adapter(),
        &mut shell("printf '\\377out\\000\\n'; printf '\\376err\\000\\n' >&2; exit 7"),
        None,
    )
    .unwrap();

    assert_eq!(result.status.code(), Some(7));
    assert_eq!(result.stdout, b"\xffout\0\n");
    assert_eq!(result.stderr, b"\xfeerr\0\n");
}

#[test]
fn streamed_completion_preserves_raw_bytes_and_nonzero_status_in_consumer_state() {
    let consumed = streaming(
        Harness::Grok.adapter(),
        &mut shell("printf 'unknown\\n\\377malformed\\n'; exit 9"),
        None,
        (Vec::new(), "initial"),
        |pipe, state| {
            state.0 = read(pipe)?;
            state.1 = "finished";
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(consumed.execution.unwrap().code(), Some(9));
    assert_eq!(
        consumed.state,
        (b"unknown\n\xffmalformed\n".to_vec(), "finished")
    );
}

#[test]
fn captured_prompt_is_null_when_absent_and_written_once_then_closed_when_present() {
    for (input, expected) in [
        (None, "nullclosed"),
        (Some(""), "closed"),
        (Some("once"), "onceclosed"),
    ] {
        let result = output(
            Harness::OpenCode.adapter(),
            &mut shell("if [ /dev/fd/0 -ef /dev/null ]; then printf null; fi; cat; printf closed"),
            input,
        )
        .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, expected.as_bytes());
    }
}

/// Cleanup uses only this fixture's recorded PID, including on assertion failure.
struct RecordedProcess(TempDir);

impl RecordedProcess {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }

    fn command(&self, script: &str) -> Command {
        let mut command = shell(script);
        command.env("PID_FILE", self.0.path().join("pid"));
        command
    }

    fn pid(&self) -> libc::pid_t {
        fs::read_to_string(self.0.path().join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    fn assert_stopped(&self) {
        assert!(!exists(self.pid()), "the owned child survived cleanup");
    }
}

fn exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 probes only the exact PID recorded by this test.
    unsafe { libc::kill(pid, 0) == 0 }
}

impl Drop for RecordedProcess {
    fn drop(&mut self) {
        if let Ok(pid) = fs::read_to_string(self.0.path().join("pid"))
            && let Ok(pid) = pid.trim().parse()
            && exists(pid)
        {
            // SAFETY: this exact PID belongs to the test's finite command.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

#[test]
fn broken_prompt_stops_and_reaps_the_child_with_the_original_stdin_error() {
    let child = RecordedProcess::new();
    let started = Instant::now();
    let result = output(
        Harness::OpenCode.adapter(),
        &mut child.command("echo $$ > \"$PID_FILE\"; exec 0<&-; exec sleep 5"),
        Some(&"p".repeat(1024 * 1024)),
    );

    let error = result.unwrap_err();
    assert!(
        format!("{error:#}").contains("could not write the prompt to opencode"),
        "{error:#}"
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    child.assert_stopped();
}

struct FailingLog;

impl Write for FailingLog {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("log device failed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn consumer_log_failure_cleans_up_even_with_a_blocked_prompt_writer() {
    let child = RecordedProcess::new();
    let escaped = RecordedProcess::new();
    let mut command = child.command(
        "set -m; trap 'wait; exit' TERM; echo $$ > \"$PID_FILE\"; \
         sleep 5 </dev/null >/dev/null 2>&1 & echo $! > \"$DESCENDANT_PID_FILE\"; \
         printf ready; wait",
    );
    command.env("DESCENDANT_PID_FILE", escaped.0.path().join("pid"));
    let started = Instant::now();
    let result = streaming(
        Harness::OpenCode.adapter(),
        &mut command,
        Some(&"p".repeat(1024 * 1024)),
        b"initial".to_vec(),
        |mut pipe, state| {
            let mut bytes = [0; 5];
            pipe.read_exact(&mut bytes)?;
            state.extend_from_slice(&bytes);
            FailingLog
                .write_all(&bytes)
                .context("could not write Session log")
        },
    )
    .unwrap();

    assert_eq!(result.state, b"initialready");
    assert_eq!(
        format!("{:#}", result.execution.unwrap_err()),
        "could not write Session log: log device failed"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    child.assert_stopped();
    escaped.assert_stopped();
}

#[test]
fn prompt_failure_retains_state_and_precedes_a_consumer_failure_during_cleanup() {
    let child = RecordedProcess::new();
    let result = streaming(
        Harness::OpenCode.adapter(),
        &mut child.command("echo $$ > \"$PID_FILE\"; printf ready; exec 0<&-; exec sleep 5"),
        Some(&"p".repeat(1024 * 1024)),
        b"initial".to_vec(),
        |mut pipe, state| {
            pipe.read_to_end(state)?;
            bail!("later consumer failure")
        },
    )
    .unwrap();

    assert_eq!(result.state, b"initialready");
    let error = result.execution.unwrap_err();
    assert_eq!(error.to_string(), "could not write the prompt to opencode");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    child.assert_stopped();
}

#[test]
fn consumer_panic_stops_and_reaps_the_child_and_reports_its_cause() {
    let child = RecordedProcess::new();
    let started = Instant::now();
    let result = streaming::<()>(
        Harness::OpenCode.adapter(),
        &mut child.command("echo $$ > \"$PID_FILE\"; printf ready; exec sleep 5"),
        None,
        (),
        |mut pipe, _state| {
            pipe.read_exact(&mut [0; 5])?;
            panic!("consumer exploded");
        },
    );

    assert_eq!(
        format!("{:#}", result.unwrap_err()),
        "the session stream reader panicked: consumer exploded"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    child.assert_stopped();
}

#[test]
fn unread_captured_prompt_remains_interruptible() {
    assert_unread_prompt_interruptible(
        "harness::process::tests::unread_captured_prompt_remains_interruptible",
        true,
    );
}

#[test]
fn unread_streamed_prompt_remains_interruptible() {
    assert_unread_prompt_interruptible(
        "harness::process::tests::unread_streamed_prompt_remains_interruptible",
        false,
    );
}

#[test]
fn eof_does_not_finish_either_operation_before_the_child_exits() {
    for captured in [true, false] {
        let mut command = shell("exec 1>&- 2>&-; sleep 0.3; exit 7");
        let started = Instant::now();
        let status = if captured {
            output(Harness::OpenCode.adapter(), &mut command, None)
                .unwrap()
                .status
        } else {
            streaming(
                Harness::OpenCode.adapter(),
                &mut command,
                None,
                Vec::new(),
                |pipe, state| {
                    *state = read(pipe)?;
                    Ok(())
                },
            )
            .unwrap()
            .execution
            .unwrap()
        };
        assert_eq!(status.code(), Some(7));
        assert!(started.elapsed() >= Duration::from_millis(250));
    }
}

#[test]
fn both_operations_wait_for_output_held_after_child_exit() {
    for captured in [true, false] {
        let mut command = shell("(sleep 0.3; printf late) & exit 7");
        let (status, bytes) = if captured {
            let result = output(Harness::OpenCode.adapter(), &mut command, None).unwrap();
            (result.status, result.stdout)
        } else {
            let result = streaming(
                Harness::OpenCode.adapter(),
                &mut command,
                None,
                Vec::new(),
                |pipe, state| {
                    *state = read(pipe)?;
                    Ok(())
                },
            )
            .unwrap();
            (result.execution.unwrap(), result.state)
        };
        assert_eq!(status.code(), Some(7));
        assert_eq!(bytes, b"late");
    }
}

#[test]
fn normal_completion_leaves_background_work_without_owned_io_running() {
    for captured in [true, false] {
        let work = RecordedProcess::new();
        let mut command =
            work.command("sleep 5 </dev/null >/dev/null 2>&1 & echo $! > \"$PID_FILE\"; exit 0");
        let status = if captured {
            output(Harness::OpenCode.adapter(), &mut command, None)
                .unwrap()
                .status
        } else {
            streaming(
                Harness::OpenCode.adapter(),
                &mut command,
                None,
                (),
                |pipe, _state| {
                    read(pipe)?;
                    Ok(())
                },
            )
            .unwrap()
            .execution
            .unwrap()
        };

        assert!(status.success());
        assert!(
            exists(work.pid()),
            "normal completion killed background work"
        );
    }
}

#[test]
fn captured_completion_waits_for_stderr_after_stdout_and_child_exit() {
    let result = output(
        Harness::OpenCode.adapter(),
        &mut shell("(exec 1>&-; sleep 0.3; printf late-diagnostic >&2) & exit 7"),
        None,
    )
    .unwrap();

    assert_eq!(result.status.code(), Some(7));
    assert!(result.stdout.is_empty());
    assert_eq!(result.stderr, b"late-diagnostic");
}

fn assert_unread_prompt_interruptible(test_name: &str, captured: bool) {
    const SIGNAL_DIR: &str = "THIRDSHIFT_PROCESS_TEST_SIGNAL_DIR";
    // Run this interface test alone in another process: the interruption
    // predicate is process-wide and must not affect concurrent unit tests.
    if let Some(dir) = std::env::var_os(SIGNAL_DIR) {
        interrupt::install().unwrap();
        let owned = RecordedProcess::new();
        let mut command =
            owned.command("echo $$ > \"$PID_FILE\"; touch \"$READY_FILE\"; exec sleep 5");
        command.env("READY_FILE", Path::new(&dir).join("ready"));
        let prompt = "p".repeat(1024 * 1024);
        let result = if captured {
            output(Harness::OpenCode.adapter(), &mut command, Some(&prompt)).map(|_| ())
        } else {
            streaming(
                Harness::OpenCode.adapter(),
                &mut command,
                Some(&prompt),
                Vec::new(),
                |pipe, state| {
                    *state = read(pipe)?;
                    Ok(())
                },
            )
            .map(|_| ())
        };
        assert_eq!(format!("{:#}", result.unwrap_err()), "interrupted");
        owned.assert_stopped();
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(SIGNAL_DIR, dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("ready").exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !dir.path().join("ready").exists() {
        let _ = child.kill();
        let result = child.wait_with_output().unwrap();
        panic!("the isolated test never started: {result:?}");
    }
    let interrupted_at = Instant::now();
    // SAFETY: this is the exact PID of our isolated test process.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(
        interrupted_at.elapsed() < Duration::from_secs(2),
        "the unread prompt prevented interruption"
    );
}
