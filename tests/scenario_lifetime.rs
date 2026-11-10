//! What a `Scenario`'s Runs start must end with the scenario, even when its
//! test harness is killed. A supervisor survives the harness and, as a Linux
//! subreaper, adopts whatever outlives it, so each check names exact recorded
//! processes rather than depending on PID 1.

mod support;

use std::fs;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use support::{Scenario, WAIT_BOUND, write_executable};
use tempfile::TempDir;

const BOUND: Duration = Duration::from_secs(5);

/// The processes a held Run records, its guardian last since it exits only
/// after reaping the rest.
const AGENT_OWNED: [&str; 5] = ["thirdshift", "agent", "script", "detached", "guardian"];
const GIT_OWNED: [&str; 3] = ["thirdshift", "git", "guardian"];

struct Fixture {
    harness: Child,
    root: TempDir,
    owned: Vec<(&'static str, libc::pid_t)>,
}

impl Fixture {
    fn start(mode: &str, owned: &[&'static str]) -> Self {
        // SAFETY: adoption is process-wide, but all waits name only this
        // fixture's recorded processes, never another parallel test's.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
        let root = TempDir::new().unwrap();
        let output = fs::File::create(root.path().join("output")).unwrap();
        let harness = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fixture_process", "--ignored", "--nocapture"])
            .env("SCENARIO_LIFETIME_MODE", mode)
            .env("SCENARIO_LIFETIME_ROOT", root.path())
            // The supervisor owns the scenario's temp root even if SIGKILL
            // keeps the fixture from dropping it.
            .env("TMPDIR", root.path())
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            // A group of its own, so killing it spares the supervisor.
            .process_group(0)
            .spawn()
            .unwrap();
        let mut fixture = Self {
            harness,
            root,
            owned: Vec::new(),
        };
        let deadline = Instant::now() + WAIT_BOUND;
        while !fixture.root.path().join("ready").exists() {
            assert!(
                fixture.harness.try_wait().unwrap().is_none(),
                "fixture exited before its Run was held: {}",
                fixture.output()
            );
            assert!(
                Instant::now() < deadline,
                "the fixture's Run was never held: {}",
                fixture.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
        fixture.owned = owned
            .iter()
            .map(|&name| {
                let pid = fs::read_to_string(fixture.root.path().join(name)).unwrap();
                (name, pid.trim().parse().unwrap())
            })
            .collect();
        fixture
    }

    fn output(&self) -> String {
        fs::read_to_string(self.root.path().join("output")).unwrap()
    }

    fn finish(&mut self, success: bool) {
        fs::write(self.root.path().join("continue"), "").unwrap();
        let deadline = Instant::now() + BOUND;
        loop {
            if let Some(status) = self.harness.try_wait().unwrap() {
                assert_eq!(status.success(), success, "{}", self.output());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture exceeded {BOUND:?}: {}",
                self.output()
            );
            thread::sleep(Duration::from_millis(10));
        }
        self.assert_owned_gone();
    }

    /// SIGKILL the harness alone, as when only its process is killed.
    fn kill_harness(&mut self) {
        self.harness.kill().unwrap();
        assert_eq!(self.harness.wait().unwrap().signal(), Some(libc::SIGKILL));
        self.assert_owned_gone();
    }

    /// SIGKILL the harness's whole process group, as an interrupted suite run
    /// dies.
    fn kill_harness_group(&mut self) {
        // SAFETY: the group is the unreaped harness's own.
        assert_eq!(
            unsafe { libc::kill(-(self.harness.id() as libc::pid_t), libc::SIGKILL) },
            0
        );
        assert_eq!(self.harness.wait().unwrap().signal(), Some(libc::SIGKILL));
        self.assert_owned_gone();
    }

    fn assert_owned_gone(&mut self) {
        for &(name, pid) in &self.owned {
            let deadline = Instant::now() + BOUND;
            loop {
                let mut status = 0;
                // SAFETY: only reap the exact process recorded by this fixture.
                match unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } {
                    found if found == pid => break,
                    -1 => {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
                        // Not adopted here: gone, or reaped by its guardian.
                        if unsafe { libc::kill(pid, 0) } == -1 {
                            assert_eq!(
                                std::io::Error::last_os_error().raw_os_error(),
                                Some(libc::ESRCH)
                            );
                            break;
                        }
                    }
                    0 => {}
                    result => panic!("unexpected waitpid result: {result}"),
                }
                assert!(
                    Instant::now() < deadline,
                    "{name} {pid} survived its fixture: {}",
                    self.output()
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        self.owned.clear();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Emergency cleanup bounds a regression in the guardian.
        if self.harness.try_wait().ok().flatten().is_none() {
            // SAFETY: the group is the unreaped harness's own.
            unsafe { libc::kill(-(self.harness.id() as libc::pid_t), libc::SIGKILL) };
        }
        let _ = self.harness.wait();
        for &(_, pid) in &self.owned {
            let mut status = 0;
            // SAFETY: only a process adopted here and still unreaped is
            // killed, so its pid cannot have been reused; one its guardian
            // reaped is left alone.
            unsafe {
                if libc::waitpid(pid, &mut status, libc::WNOHANG) == 0 {
                    libc::kill(pid, libc::SIGKILL);
                    libc::waitpid(pid, &mut status, 0);
                }
            }
        }
    }
}

#[test]
fn a_held_agent_and_its_detached_command_end_after_harness_sigkill() {
    Fixture::start("agent", &AGENT_OWNED).kill_harness();
}

#[test]
fn a_held_agent_and_its_detached_command_end_after_harness_group_sigkill() {
    Fixture::start("agent", &AGENT_OWNED).kill_harness_group();
}

#[test]
fn a_held_git_ends_after_harness_sigkill() {
    Fixture::start("git", &GIT_OWNED).kill_harness();
}

#[test]
fn a_held_git_ends_after_harness_group_sigkill() {
    Fixture::start("git", &GIT_OWNED).kill_harness_group();
}

#[test]
fn a_held_run_ends_after_normal_completion() {
    Fixture::start("complete", &AGENT_OWNED).finish(true);
}

#[test]
fn a_held_run_ends_after_assertion_unwinding() {
    let mut fixture = Fixture::start("assertion", &AGENT_OWNED);
    fixture.finish(false);
    assert!(fixture.output().contains("intentional fixture assertion"));
}

/// Record `name`'s pid in the supervisor's root, by the shell expression
/// `pid`.
fn record(root: &Path, name: &str, pid: &str) -> String {
    format!("echo {pid} > \"{}\"\n", root.join(name).display())
}

fn parent_of(pid: &str) -> String {
    format!("$(ps -o ppid= -p {pid} | tr -d ' ')")
}

#[test]
#[ignore = "subprocess entry point for Scenario lifetime tests"]
fn fixture_process() {
    let root = PathBuf::from(std::env::var_os("SCENARIO_LIFETIME_ROOT").unwrap());
    let mode = std::env::var("SCENARIO_LIFETIME_MODE").unwrap();
    let scenario = Scenario::new();
    let started = scenario.path("started");
    if mode == "git" {
        let git = Command::new("which").arg("git").output().unwrap();
        assert!(git.status.success());
        let git = String::from_utf8(git.stdout).unwrap();
        // The first git thirdshift starts holds with no exit condition.
        write_executable(
            &scenario.path("bin/git"),
            &format!(
                "#!/bin/sh\nif mkdir \"{held}\" 2>/dev/null; then\n{git_pid}{thirdshift}{guardian}touch \"{started}\"\nwhile :; do sleep 0.02; done\nfi\nexec \"{real}\" \"$@\"\n",
                held = scenario.path("git-held").display(),
                git_pid = record(&root, "git", "$$"),
                thirdshift = record(&root, "thirdshift", "$PPID"),
                guardian = record(&root, "guardian", &parent_of("$PPID")),
                started = started.display(),
                real = git.trim(),
            ),
        );
    } else {
        // The agent holds with no exit condition, after starting a detached
        // command in a session of its own that ignores INT and TERM.
        scenario.agent_does(&format!(
            "{script}{agent}{thirdshift}{guardian}detached-command bash -c 'trap \"\" INT TERM; {detached}exec sleep 86400' >/dev/null 2>&1 &\nwhile [ ! -s \"{detached_pid}\" ]; do sleep 0.01; done\ntouch \"{started}\"\nwhile :; do sleep 0.1 || :; done\n",
            script = record(&root, "script", "$$"),
            agent = record(&root, "agent", "$PPID"),
            thirdshift = record(&root, "thirdshift", &parent_of("$PPID")),
            guardian = record(&root, "guardian", &parent_of(&parent_of("$PPID"))),
            detached = record(&root, "detached", "$$").replace('\n', "; "),
            detached_pid = root.join("detached").display(),
            started = started.display(),
        ));
    }
    let _held = scenario.run_until(
        &["harness", "codex", &scenario.issue_url(7)],
        &[],
        "started",
    );
    fs::write(root.join("ready"), "").unwrap();
    let deadline = Instant::now() + WAIT_BOUND;
    while !root.join("continue").exists() {
        assert!(
            Instant::now() < deadline,
            "supervisor never released fixture"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(mode, "assertion", "intentional fixture assertion");
}
