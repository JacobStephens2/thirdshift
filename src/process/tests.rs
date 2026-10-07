//! Behavior through the same two operations used by sessions and checks.

use super::*;

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
        &mut shell("printf '\\377out\\000\\n'; printf '\\376err\\000\\n' >&2; exit 7"),
        None,
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
    )
    .unwrap();

    assert_eq!(result.status.code(), Some(7));
    assert_eq!(result.stdout, b"\xffout\0\n");
    assert_eq!(result.stderr, b"\xfeerr\0\n");
}

#[test]
fn streamed_completion_preserves_raw_bytes_and_nonzero_status_in_consumer_state() {
    let consumed = streaming(
        &mut shell("printf 'unknown\\n\\377malformed\\n'; exit 9"),
        None,
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
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
            &mut shell("if [ /dev/fd/0 -ef /dev/null ]; then printf null; fi; cat; printf closed"),
            input.map(str::as_bytes),
            "fixture",
            Interruption::Ordinary,
            &stop_fixture,
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
        &mut child.command("echo $$ > \"$PID_FILE\"; exec 0<&-; exec sleep 5"),
        Some("p".repeat(1024 * 1024).as_bytes()),
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
    );

    let error = result.unwrap_err();
    assert!(
        format!("{error:#}").contains("could not write the prompt to fixture"),
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
        &mut command,
        Some("p".repeat(1024 * 1024).as_bytes()),
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
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
        &mut child.command("echo $$ > \"$PID_FILE\"; printf ready; exec 0<&-; exec sleep 5"),
        Some("p".repeat(1024 * 1024).as_bytes()),
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
        b"initial".to_vec(),
        |mut pipe, state| {
            pipe.read_to_end(state)?;
            bail!("later consumer failure")
        },
    )
    .unwrap();

    assert_eq!(result.state, b"initialready");
    let error = result.execution.unwrap_err();
    assert_eq!(error.to_string(), "could not write the prompt to fixture");
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
        &mut child.command("echo $$ > \"$PID_FILE\"; printf ready; exec sleep 5"),
        None,
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
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
        "process::tests::unread_captured_prompt_remains_interruptible",
        true,
    );
}

#[test]
fn unread_streamed_prompt_remains_interruptible() {
    assert_unread_prompt_interruptible(
        "process::tests::unread_streamed_prompt_remains_interruptible",
        false,
    );
}

#[test]
fn eof_does_not_finish_either_operation_before_the_child_exits() {
    for captured in [true, false] {
        let mut command = shell("exec 1>&- 2>&-; sleep 0.3; exit 7");
        let started = Instant::now();
        let status = if captured {
            output(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
            )
            .unwrap()
            .status
        } else {
            streaming(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
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
            let result = output(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
            )
            .unwrap();
            (result.status, result.stdout)
        } else {
            let result = streaming(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
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
            output(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
            )
            .unwrap()
            .status
        } else {
            streaming(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
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
        &mut shell("(exec 1>&-; sleep 0.3; printf late-diagnostic >&2) & exit 7"),
        None,
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
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
            output(
                &mut command,
                Some(prompt.as_bytes()),
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
            )
            .map(|_| ())
        } else {
            streaming(
                &mut command,
                Some(prompt.as_bytes()),
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
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

fn stop_fixture(child: &mut Child) {
    stop(child, &[libc::SIGTERM]);
}

#[test]
fn completion_ignores_recorded_signals_without_permitting_ordinary_work() {
    const SIGNAL: &str = "THIRDSHIFT_EXECUTION_RECORDED_SIGNAL";
    if let Ok(signal) = std::env::var(SIGNAL) {
        interrupt::install().unwrap();
        signal_hook::low_level::raise(signal.parse().unwrap()).unwrap();
        let result = output(
            &mut shell("cat; printf diagnostic >&2; exit 7"),
            Some(b"\xffonce\0"),
            "fixture",
            Interruption::Completion,
            &stop_fixture,
        )
        .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout, b"\xffonce\0");
        assert_eq!(result.stderr, b"diagnostic");
        assert!(interrupt::requested());

        let streamed = streaming(
            &mut shell("printf retained; exec 0<&-; exec sleep 0.2"),
            Some(&vec![b'p'; 1024 * 1024]),
            "fixture",
            Interruption::Completion,
            &stop_fixture,
            Vec::new(),
            |mut pipe, state| {
                pipe.read_to_end(state)?;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(streamed.state, b"retained");
        assert_eq!(
            streamed
                .execution
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
        let panicked = streaming(
            &mut shell("printf ready; exec sleep 0.2"),
            None,
            "fixture",
            Interruption::Completion,
            &stop_fixture,
            (),
            |mut pipe, _| {
                pipe.read_exact(&mut [0; 5])?;
                panic!("completion consumer exploded")
            },
        )
        .unwrap_err();
        assert!(
            panicked
                .to_string()
                .contains("completion consumer exploded")
        );
        assert!(interrupt::requested());

        let refused = RecordedProcess::new();
        let result = output(
            &mut refused.command("echo $$ > \"$PID_FILE\""),
            None,
            "fixture",
            Interruption::Ordinary,
            &stop_fixture,
        );
        assert_eq!(result.unwrap_err().to_string(), "interrupted");
        assert!(!refused.0.path().join("pid").exists());
        return;
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::completion_ignores_recorded_signals_without_permitting_ordinary_work",
                "--nocapture",
            ])
            .env(SIGNAL, signal.to_string())
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
    }
}

#[test]
#[ignore = "local short-capture latency probe; no timing assertion for CI"]
fn representative_short_captures() {
    for executable in ["true", "bash"] {
        let started = Instant::now();
        for _ in 0..25 {
            let mut command = Command::new(executable);
            if executable == "bash" {
                command.args(["-c", "printf short"]);
            }
            let result = output(
                &mut command,
                None,
                executable,
                Interruption::Ordinary,
                &stop_fixture,
            )
            .unwrap();
            assert!(result.status.success());
            if executable == "bash" {
                assert_eq!(result.stdout, b"short");
            }
        }
        eprintln!(
            "{executable}: 25 captures in {:?} (mean {:?})",
            started.elapsed(),
            started.elapsed() / 25
        );
    }
}

#[test]
fn startup_failure_returns_no_state_even_when_the_consumer_has_returned() {
    let owned = RecordedProcess::new();
    let gate = owned.0.path().join("consumer-returned");
    let mut command = shell("printf ready; exec sleep 0.2");
    command.env("THIRDSHIFT_EXECUTION_TEST_FAULT", "stdin-start");
    command.env("THIRDSHIFT_EXECUTION_TEST_GATE", &gate);
    let stop = |child: &mut Child| {
        fs::write(owned.0.path().join("pid"), child.id().to_string()).unwrap();
        stop_fixture(child);
    };
    let result = streaming(
        &mut command,
        Some(b"prompt"),
        "fixture",
        Interruption::Ordinary,
        &stop,
        Vec::new(),
        move |mut pipe, state| {
            let mut bytes = [0; 5];
            pipe.read_exact(&mut bytes)?;
            state.extend_from_slice(&bytes);
            fs::write(gate, "returned")?;
            Ok(())
        },
    );
    assert!(
        result.is_err(),
        "startup failure returned recovered state: {result:?}"
    );
    owned.assert_stopped();
}

fn with_recorded_signal(test_name: &str, exercise: impl FnOnce(libc::c_int)) {
    const SIGNAL: &str = "THIRDSHIFT_EXECUTION_ACTIVE_SIGNAL";
    if let Ok(signal) = std::env::var(SIGNAL) {
        interrupt::install().unwrap();
        exercise(signal.parse().unwrap());
        return;
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(SIGNAL, signal.to_string())
            .output()
            .unwrap();
        assert!(result.status.success(), "{result:?}");
    }
}

#[test]
fn active_ordinary_cancellation_does_not_cancel_concurrent_completion() {
    with_recorded_signal(
        "process::tests::active_ordinary_cancellation_does_not_cancel_concurrent_completion",
        |signal| {
            let owned = RecordedProcess::new();
            let ready = owned.0.path().join("completion-ready");
            let mut completion = shell("touch \"$READY\"; sleep 0.3; printf completed; exit 7");
            completion.env("READY", &ready);
            let finishing = thread::spawn(move || {
                output(
                    &mut completion,
                    None,
                    "completion fixture",
                    Interruption::Completion,
                    &stop_fixture,
                )
            });
            let mut ordinary = owned.command("while ! test -f \"$READY\"; do sleep 0.01; done; echo $$ > \"$PID_FILE\"; kill -\"$SIGNAL\" \"$SUPERVISOR\"; exec sleep 5");
            ordinary
                .env("READY", &ready)
                .env("SIGNAL", signal.to_string())
                .env("SUPERVISOR", std::process::id().to_string());
            let started = Instant::now();
            let result = output(
                &mut ordinary,
                Some(&vec![b'p'; 1024 * 1024]),
                "ordinary fixture",
                Interruption::Ordinary,
                &stop_fixture,
            );
            // Join this exact concurrent operation before any assertion can unwind.
            let completed = finishing.join().unwrap().unwrap();
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            owned.assert_stopped();
            assert_eq!(completed.status.code(), Some(7));
            assert_eq!(completed.stdout, b"completed");
            assert!(interrupt::requested());
            assert!(started.elapsed() < Duration::from_secs(2));
        },
    );
}

#[test]
fn interruption_after_cli_exit_stops_ordinary_pipe_holders_but_completion_drains_them() {
    with_recorded_signal(
        "process::tests::interruption_after_cli_exit_stops_ordinary_pipe_holders_but_completion_drains_them",
        |signal| {
            // Completion receives the new signal while its direct CLI has exited.
            // The following ordinary call observes that same unchanged flag.
            for policy in [Interruption::Completion, Interruption::Ordinary] {
                let holder = RecordedProcess::new();
                let mut command = holder.command("bash -c 'while kill -0 \"$1\" 2>/dev/null; do sleep 0.01; done; echo $$ > \"$PID_FILE\"; kill -\"$SIGNAL\" \"$SUPERVISOR\"; sleep 0.2; printf late' holder $$ & exit 7");
                command
                    .env("SIGNAL", signal.to_string())
                    .env("SUPERVISOR", std::process::id().to_string());
                let result = output(&mut command, None, "fixture", policy, &stop_fixture);
                if matches!(policy, Interruption::Completion) {
                    let result = result.unwrap();
                    assert_eq!(result.status.code(), Some(7));
                    assert_eq!(result.stdout, b"late");
                } else {
                    assert_eq!(result.unwrap_err().to_string(), "interrupted");
                    assert!(!holder.0.path().join("pid").exists());
                }
                assert!(interrupt::requested());
            }
        },
    );
}

#[test]
fn active_ordinary_cancellation_stops_output_held_after_cli_exit() {
    with_recorded_signal(
        "process::tests::active_ordinary_cancellation_stops_output_held_after_cli_exit",
        |signal| {
            let holder = RecordedProcess::new();
            let mut command = holder.command("bash -c 'while kill -0 \"$1\" 2>/dev/null; do sleep 0.01; done; echo $$ > \"$PID_FILE\"; kill -\"$SIGNAL\" \"$SUPERVISOR\"; exec sleep 5' holder $$ & exit 7");
            command
                .env("SIGNAL", signal.to_string())
                .env("SUPERVISOR", std::process::id().to_string());
            let started = Instant::now();
            let result = output(
                &mut command,
                None,
                "fixture",
                Interruption::Ordinary,
                &stop_fixture,
            );
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            assert!(started.elapsed() < Duration::from_secs(2));
            holder.assert_stopped();
        },
    );
}

#[test]
fn captured_execution_drains_full_pipes_concurrently_with_binary_stdin() {
    let result = output(
        &mut shell("head -c 262144 /dev/zero & head -c 262144 /dev/zero >&2 & cat >/dev/null; wait; printf drained"),
        Some(&vec![0xff; 262144]),
        "fixture",
        Interruption::Ordinary,
        &stop_fixture,
    ).unwrap();
    assert!(result.status.success());
    let mut stdout = vec![0; 262144];
    stdout.extend_from_slice(b"drained");
    assert_eq!(result.stdout, stdout);
    assert_eq!(result.stderr, vec![0; 262144]);
}

#[test]
fn captured_setup_and_worker_faults_stop_reap_and_join_before_return() {
    for fault in [
        "stdout-pipe",
        "stdout-start",
        "stderr-pipe",
        "stderr-start",
        "stdin-pipe",
        "stdin-start",
        "stdout-read",
        "stderr-read",
        "stdin-write",
        "stdout-read-panic",
        "stderr-read-panic",
        "stdin-write-panic",
        "stderr-start-panic",
        "wait-panic",
    ] {
        let owned = RecordedProcess::new();
        let mut command = shell("exec sleep 0.2");
        command.env("THIRDSHIFT_EXECUTION_TEST_FAULT", fault);
        let stop = |child: &mut Child| {
            fs::write(owned.0.path().join("pid"), child.id().to_string()).unwrap();
            stop_fixture(child);
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            output(
                &mut command,
                Some(b"prompt"),
                "fixture",
                Interruption::Ordinary,
                &stop,
            )
        }));
        assert!(
            matches!(result, Err(_) | Ok(Err(_))),
            "{fault} returned success"
        );
        owned.assert_stopped();
    }
}

#[test]
fn interruption_during_cleanup_suppresses_retained_state_and_transport_failure() {
    with_recorded_signal(
        "process::tests::interruption_during_cleanup_suppresses_retained_state_and_transport_failure",
        |signal| {
            let owned = RecordedProcess::new();
            let stop = |child: &mut Child| {
                signal_hook::low_level::raise(signal).unwrap();
                stop_fixture(child);
            };
            let result = streaming(
                &mut owned
                    .command("echo $$ > \"$PID_FILE\"; printf ready; exec 0<&-; exec sleep 5"),
                Some(&vec![b'p'; 1024 * 1024]),
                "fixture",
                Interruption::Ordinary,
                &stop,
                Vec::new(),
                |mut pipe, state| {
                    pipe.read_to_end(state)?;
                    Ok(())
                },
            );
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            owned.assert_stopped();
        },
    );
}

#[test]
fn spawn_failure_returns_no_streamed_state_and_names_the_command() {
    let result = streaming(
        &mut Command::new("/thirdshift-no-such-command-fixture"),
        None,
        "missing fixture",
        Interruption::Ordinary,
        &stop_fixture,
        "initial",
        |_, _| panic!("consumer must not run after spawn failure"),
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "could not run missing fixture"
    );
}
