//! Interrupting a Run stops commands that escaped the session's process group.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::thread;
use std::time::{Duration, Instant};

use support::Scenario;
use support::check::{DuringCheck, OwnedCheck};

#[test]
fn interrupting_a_live_claude_model_check_stops_before_work() {
    assert_owned_check_stopped("claude", "opus", DuringCheck::Live, "INT");
}

#[test]
fn interrupting_a_live_codex_catalog_stops_before_work() {
    assert_owned_check_stopped("codex", "gpt-6.1-sol", DuringCheck::Live, "TERM");
}

#[test]
fn interrupting_a_live_agy_catalog_stops_before_work() {
    assert_owned_check_stopped("agy", "gemini-3.8-flash-high", DuringCheck::Live, "HUP");
}

#[test]
fn interrupting_a_live_grok_catalog_stops_before_work() {
    assert_owned_check_stopped("grok", "grok-4.7", DuringCheck::Live, "INT");
}

#[test]
fn interrupting_each_migrated_check_stops_its_owned_detached_descendant() {
    for (harness, model, signal) in [
        ("claude", "opus", "TERM"),
        ("codex", "gpt-6.1-sol", "HUP"),
        ("agy", "gemini-3.8-flash-high", "INT"),
        ("grok", "grok-4.7", "TERM"),
    ] {
        assert_owned_check_stopped(harness, model, DuringCheck::Detached, signal);
    }
}

#[test]
fn interrupting_each_migrated_check_after_cli_exit_stops_its_pipe_holder() {
    for (harness, model, signal) in [
        ("claude", "opus", "HUP"),
        ("codex", "gpt-6.1-sol", "INT"),
        ("agy", "gemini-3.8-flash-high", "TERM"),
        ("grok", "grok-4.7", "HUP"),
    ] {
        assert_owned_check_stopped(harness, model, DuringCheck::AfterExit, signal);
    }
}

fn assert_owned_check_stopped(harness: &str, model: &str, during: DuringCheck, signal: &str) {
    let scenario = Scenario::new();
    let check = OwnedCheck::new(&scenario, during);
    let mut held = scenario.run_until(
        &["harness", harness, "model", model, &scenario.issue_url(7)],
        &check.env(),
        "check-started",
    );
    let started = Instant::now();
    held.signal(signal);
    let result = held.finish();

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{}",
        result.stderr
    );
    check.assert_stopped();
    assert!(scenario.issue_labels(7).is_empty(), "the Claim was made");
    assert_eq!(scenario.entries("work"), ["widgets"]);
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

/// Clean up by the recorded pid even when the regression assertion fails.
struct SessionCommand(libc::pid_t);

enum During {
    Session,
    Check,
    CheckAfterExit,
    CheckExport,
    Export,
}

impl SessionCommand {
    fn exists(&self) -> bool {
        // SAFETY: kill has no memory-safety preconditions; signal 0 only probes.
        unsafe { libc::kill(self.0, 0) == 0 }
    }

    fn wait_for_stop(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        !self.exists()
    }

    fn assert_stopped(&self, message: &str) {
        assert!(self.wait_for_stop(), "{message}");
    }
}

impl Drop for SessionCommand {
    fn drop(&mut self) {
        if self.exists() {
            // SAFETY: the recorded pid belongs to the command. Stop its group
            // when it made one, and the command itself when it inherited one.
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
                libc::kill(self.0, libc::SIGKILL);
            }
        }
    }
}

#[test]
fn interrupting_a_claude_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("claude", During::Session);
}

#[test]
fn interrupting_a_codex_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("codex", During::Session);
}

#[test]
fn interrupting_an_agy_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("agy", During::Session);
}

#[test]
fn interrupting_an_opencode_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("opencode", During::Session);
}

#[test]
fn interrupting_an_opencode_check_stops_its_whole_process_tree_before_work() {
    assert_session_command_stopped("opencode", During::Check);
}

#[test]
fn interrupting_a_muse_check_stops_its_whole_process_tree_before_work() {
    assert_session_command_stopped("muse", During::Check);
}

#[test]
fn interrupting_a_muse_check_after_cli_exit_stops_the_command_holding_its_stream() {
    assert_session_command_stopped("muse", During::CheckAfterExit);
}

#[test]
fn interrupting_a_live_session_after_cli_exit_stops_the_command_holding_stdout() {
    assert_live_session_stdout_holder_stopped(false);
}

#[test]
fn interrupting_a_live_session_stops_its_stdout_holder_before_slow_cleanup() {
    assert_live_session_stdout_holder_stopped(true);
}

fn assert_live_session_stdout_holder_stopped(slow_cleanup: bool) {
    let scenario = Scenario::new();
    // Failed run preservation may be slow even when the session stops promptly.
    scenario.repo_has_hook(
        &scenario.origin_dir(),
        "pre-receive",
        "#!/bin/sh\nsleep 3\n",
    );
    let pid = scenario.path("stdout-holder-pid");
    let cleanup_started = scenario.path("cleanup-started");
    let holder_at_cleanup = scenario.path("stdout-holder-at-cleanup");
    if slow_cleanup {
        // Scenario prepends bin to PATH; delegate past this shim after
        // delaying only the finishing worktree removal.
        let git = scenario.path("bin/git");
        fs::write(
            &git,
            format!(
                r#"#!/bin/sh
if test "$1" = worktree && test "$2" = remove; then
    touch "{cleanup_started}"
    if read -r holder < "{pid}" && kill -0 "$holder" 2>/dev/null; then
        touch "{holder_at_cleanup}"
    fi
    sleep 3
fi
PATH="${{PATH#*:}}" exec git "$@"
"#,
                cleanup_started = cleanup_started.display(),
                pid = pid.display(),
                holder_at_cleanup = holder_at_cleanup.display(),
            ),
        )
        .unwrap();
        fs::set_permissions(git, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let expired = scenario.path("stdout-holder-expired");
    // Bash waits on its own FIFO so a child sleep cannot outlive the holder
    // and keep stdout open without writing the natural-expiry marker.
    scenario.agent_does(&format!(
        r#"printf 'interrupted work\n' > interrupted-work.txt
bash -c 'while kill -0 "$1" 2>/dev/null; do sleep 0.01; done
mkfifo "$5"
exec 9<>"$5"
echo $$ > "$2"
touch "$3"
read -r -t 30 -u 9
touch "$4"' holder "$PPID" "{pid}" "{started}" "{expired}" "{wait_pipe}" &
"#,
        pid = pid.display(),
        started = scenario.path("cli-exited").display(),
        expired = expired.display(),
        wait_pipe = scenario.path("stdout-holder-wait").display(),
    ));
    let url = scenario.issue_url(7);
    let mut held = scenario.run_until(&[&url], &[], "cli-exited");
    let command = SessionCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());
    assert!(command.exists(), "the stdout holder never started");
    held.signal("INT");
    // Observe stopping before waiting for the Run's preservation and cleanup.
    // Finish the exact Run before asserting, so a failure leaves no Run behind.
    let stopped = command.wait_for_stop();
    let result = held.finish();

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    // Observe whether the holder expired: timing the whole Run also counts
    // finishing Git work and Claim release, which can be slow under CI load.
    assert!(
        stopped && !expired.exists(),
        "interruption waited for the stdout holder's natural expiry"
    );
    command.assert_stopped("the stdout holder survived interruption");
    assert_eq!(
        scenario
            .origin_file("issue-7", "interrupted-work.txt")
            .as_deref(),
        Some("interrupted work\n"),
        "the slow push did not preserve the interrupted work"
    );
    if slow_cleanup {
        assert!(cleanup_started.exists(), "the delayed cleanup never ran");
        assert!(
            !holder_at_cleanup.exists(),
            "the stdout holder was still running when cleanup started"
        );
    }
}

#[test]
fn interrupting_an_opencode_export_stops_its_whole_process_tree() {
    assert_session_command_stopped("opencode", During::Export);
}

#[test]
fn interrupting_an_opencode_check_export_stops_its_tree_without_a_model_refusal() {
    assert_session_command_stopped("opencode", During::CheckExport);
}

#[test]
fn interrupting_a_grok_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("grok", During::Session);
}

#[test]
fn interrupting_a_muse_run_stops_a_command_in_its_own_session() {
    assert_session_command_stopped("muse", During::Session);
}

fn assert_session_command_stopped(harness: &str, during: During) {
    let scenario = Scenario::new();
    let pid = scenario.path("detached-pid");
    let checking = matches!(
        during,
        During::Check | During::CheckAfterExit | During::CheckExport
    );
    let ending = if matches!(during, During::Session) {
        "while :; do sleep 0.1; done"
    } else {
        "sleep 9"
    };
    let interrupt_script = if matches!(during, During::CheckAfterExit) {
        // Wait for the recorded CLI to exit so interruption arrives while
        // only this command is keeping the captured output pipes open.
        format!(
            r#"while kill -0 "$FAKE_MUSE_CHECK_PID" 2>/dev/null; do sleep 0.01; done
echo $$ > "{pid}"
touch "{started}"
exec sleep 5
"#,
            pid = pid.display(),
            started = scenario.path("agent-started").display(),
        )
    } else {
        format!(
            r#"detached-command bash -c 'trap "" INT TERM; echo $$ > "{pid}"; exec sleep 120' >/dev/null 2>&1 &
while ! test -s "{pid}"; do sleep 0.01; done
touch "{started}"
{ending}
"#,
            pid = pid.display(),
            started = scenario.path("agent-started").display(),
        )
    };

    let script = scenario.path("interrupt-script.sh");
    fs::write(&script, &interrupt_script).unwrap();
    let env = match during {
        During::Session => {
            scenario.agent_does(&interrupt_script);
            Vec::new()
        }
        During::Check | During::CheckAfterExit => {
            let name = if harness == "muse" {
                "FAKE_MUSE_CHECK_SCRIPT"
            } else {
                "FAKE_OPENCODE_CHECK_SCRIPT"
            };
            let mut env = vec![(name, script.to_str().unwrap())];
            if matches!(during, During::CheckAfterExit) {
                env.push(("FAKE_MUSE_CHECK_EXIT_EARLY", "1"));
            }
            env
        }
        During::Export | During::CheckExport => {
            scenario.agent_does("true");
            let name = if matches!(during, During::CheckExport) {
                "FAKE_OPENCODE_CHECK_EXPORT_SCRIPT"
            } else {
                "FAKE_OPENCODE_EXPORT_SCRIPT"
            };
            vec![(name, script.to_str().unwrap())]
        }
    };
    let url = scenario.issue_url(7);
    let mut args = vec!["harness", harness];
    if checking && harness == "muse" {
        args.extend(["model", "muse-spark-1.3"]);
    }
    args.push(&url);
    let mut held = scenario.run_until(&args, &env, "agent-started");
    let command = SessionCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());
    assert!(command.exists(), "the command never started");
    let interrupted_at = Instant::now();
    held.signal("INT");
    let result = held.finish();

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    if checking {
        assert!(scenario.issue_labels(7).is_empty());
        assert_eq!(scenario.entries("work"), ["widgets"]);
    }
    if matches!(during, During::CheckExport) {
        assert!(
            result.stderr.ends_with("thirdshift: interrupted\n"),
            "{}",
            result.stderr
        );
        assert!(!result.stderr.contains("refused"), "{}", result.stderr);
    }
    if matches!(during, During::Export) {
        assert!(
            !result.stderr.contains("session ended after"),
            "{}",
            result.stderr
        );
        assert!(
            !result.stderr.contains("warning: the session never loaded"),
            "{}",
            result.stderr
        );
    }
    let limit = if matches!(during, During::CheckAfterExit) {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(8)
    };
    assert!(
        interrupted_at.elapsed() < limit,
        "interruption waited for the command instead of stopping it"
    );
    command.assert_stopped("the session's command survived interruption");
}

#[test]
fn a_command_started_during_the_stop_grace_period_is_killed_too() {
    let scenario = Scenario::new();
    let pid = scenario.path("late-pid");
    let signals = scenario.path("signals");
    scenario.agent_does(&format!(
        r#"on_int() {{
    echo INT >> "{signals}"
    detached-command bash -c 'trap "" INT TERM; echo $$ > "{pid}"; exec sleep 120' >/dev/null 2>&1 &
}}
trap on_int INT
trap 'echo TERM >> "{signals}"' TERM
touch "{started}"
while :; do sleep 0.1 || :; done
"#,
        pid = pid.display(),
        signals = signals.display(),
        started = scenario.path("agent-started").display(),
    ));

    let result = scenario.run_and_signal(
        &["harness", "codex", &scenario.issue_url(7)],
        "agent-started",
        "INT",
    );
    let command = SessionCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(fs::read_to_string(signals).unwrap(), "INT\nTERM\n");
    command.assert_stopped("the command started during the grace period survived");
}
