//! Interrupting a Run stops commands that escaped the session's process group.

mod support;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use support::Scenario;

/// Clean up by the recorded pid even when the regression assertion fails.
struct SessionCommand(libc::pid_t);

enum During {
    Session,
    Check,
    CheckAfterExit,
    Export,
}

impl SessionCommand {
    fn exists(&self) -> bool {
        // SAFETY: kill has no memory-safety preconditions; signal 0 only probes.
        unsafe { libc::kill(self.0, 0) == 0 }
    }

    fn assert_stopped(&self, message: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!self.exists(), "{message}");
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
    let scenario = Scenario::new();
    let pid = scenario.path("stdout-holder-pid");
    scenario.agent_does(&format!(
        r#"bash -c 'while kill -0 "$1" 2>/dev/null; do sleep 0.01; done
echo $$ > "$2"
touch "$3"
exec sleep 5' holder "$PPID" "{pid}" "{started}" &
"#,
        pid = pid.display(),
        started = scenario.path("cli-exited").display(),
    ));
    let url = scenario.issue_url(7);
    let mut held = scenario.run_until(&[&url], &[], "cli-exited");
    let command = SessionCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());
    let interrupted_at = Instant::now();
    held.signal("INT");
    let result = held.finish();

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert!(
        interrupted_at.elapsed() < Duration::from_secs(2),
        "interruption waited for the stdout holder's natural expiry"
    );
    command.assert_stopped("the stdout holder survived interruption");
}

#[test]
fn interrupting_an_opencode_export_stops_its_whole_process_tree() {
    assert_session_command_stopped("opencode", During::Export);
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
    let checking = matches!(during, During::Check | During::CheckAfterExit);
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
        During::Export => {
            scenario.agent_does("true");
            vec![("FAKE_OPENCODE_EXPORT_SCRIPT", script.to_str().unwrap())]
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
