//! Finite local Harness checks with cleanup by exact recorded owned PIDs.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use super::Scenario;

pub enum DuringCheck {
    Live,
    Detached,
    AfterExit,
}

pub struct OwnedCheck {
    script: PathBuf,
    pids: PathBuf,
    after_exit: bool,
}

impl OwnedCheck {
    pub fn new(scenario: &Scenario, during: DuringCheck) -> Self {
        let fixture = Self {
            script: scenario.path("owned-check.sh"),
            pids: scenario.path("owned-check-pids"),
            after_exit: matches!(during, DuringCheck::AfterExit),
        };
        let prepare = match during {
            DuringCheck::Live => String::new(),
            DuringCheck::Detached => format!(
                r#"detached-command bash -c 'trap "" INT TERM; echo $$ >> "$FAKE_CHECK_PIDS"; touch "{detached}"; exec sleep 9' >/dev/null 2>&1 &
while ! test -e "{detached}"; do sleep 0.01; done
"#,
                detached = scenario.path("detached-started").display(),
            ),
            DuringCheck::AfterExit => {
                "while kill -0 \"$FAKE_CHECK_CLI_PID\" 2>/dev/null; do sleep 0.01; done\n"
                    .to_string()
            }
        };
        fs::write(
            &fixture.script,
            format!(
                "echo $$ >> \"$FAKE_CHECK_PIDS\"\n{prepare}touch \"{started}\"\nprintf 'owned check ready\\n'\nexec sleep 9\n",
                started = scenario.path("check-started").display(),
            ),
        )
        .unwrap();
        fixture
    }

    pub fn env(&self) -> Vec<(&str, &str)> {
        let mut env = vec![
            ("FAKE_CHECK_SCRIPT", self.script.to_str().unwrap()),
            ("FAKE_CHECK_PIDS", self.pids.to_str().unwrap()),
        ];
        if self.after_exit {
            env.push(("FAKE_CHECK_EXIT_EARLY", "1"));
        }
        env
    }

    fn pids(&self) -> Vec<libc::pid_t> {
        fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .lines()
            .map(|pid| pid.parse().unwrap())
            .collect()
    }

    pub fn assert_stopped(&self) {
        let pids = self.pids();
        assert!(
            pids.len() >= 2,
            "the check did not record its owned processes"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while pids.iter().any(|&pid| exists(pid)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        for pid in pids {
            assert!(!exists(pid), "owned check process {pid} survived");
        }
    }
}

fn exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 probes only the exact PID recorded by this fixture.
    unsafe { libc::kill(pid, 0) == 0 }
}

impl Drop for OwnedCheck {
    fn drop(&mut self) {
        for pid in self.pids() {
            if exists(pid) {
                // SAFETY: only this fixture's exact recorded finite processes.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}
