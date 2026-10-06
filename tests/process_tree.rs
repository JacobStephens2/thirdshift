//! Interrupting a Run stops commands that escaped the session's process group.

mod support;

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use support::Scenario;

/// Clean up by the recorded pid even when the regression assertion fails.
struct DetachedCommand(libc::pid_t);

impl DetachedCommand {
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

impl Drop for DetachedCommand {
    fn drop(&mut self) {
        if self.exists() {
            // SAFETY: the recorded pid is this command's process group leader.
            unsafe { libc::kill(-self.0, libc::SIGKILL) };
        }
    }
}

#[test]
fn interrupting_a_claude_run_stops_a_command_in_its_own_session() {
    assert_detached_command_stopped("claude");
}

#[test]
fn interrupting_a_codex_run_stops_a_command_in_its_own_session() {
    assert_detached_command_stopped("codex");
}

#[test]
fn interrupting_a_grok_run_stops_a_command_in_its_own_session() {
    assert_detached_command_stopped("grok");
}

fn assert_detached_command_stopped(harness: &str) {
    let scenario = Scenario::new();
    let pid = scenario.path("detached-pid");
    scenario.agent_does(&format!(
        r#"detached-command bash -c 'trap "" INT TERM; echo $$ > "{pid}"; exec sleep 120' >/dev/null 2>&1 &
while ! test -s "{pid}"; do sleep 0.01; done
touch "{started}"
while :; do sleep 0.1; done
"#,
        pid = pid.display(),
        started = scenario.path("agent-started").display(),
    ));

    let mut held = scenario.run_until(
        &["harness", harness, &scenario.issue_url(7)],
        &[],
        "agent-started",
    );
    let command = DetachedCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());
    assert!(command.exists(), "the detached command never started");
    let interrupted_at = Instant::now();
    held.signal("INT");
    let result = held.finish();

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        interrupted_at.elapsed() < Duration::from_secs(8),
        "a session that exited on its first signal incurred another grace period"
    );
    command.assert_stopped("the detached command survived interruption");
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
    let command = DetachedCommand(fs::read_to_string(pid).unwrap().trim().parse().unwrap());

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(fs::read_to_string(signals).unwrap(), "INT\nTERM\n");
    command.assert_stopped("the command started during the grace period survived");
}
