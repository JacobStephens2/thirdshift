//! Real setup children must end with their PTY fixture. Linux's subreaper
//! lets this test reap an orphan without relying on the machine's PID 1.

#[cfg(target_os = "linux")]
mod support;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::support::{Scenario, TerminalStep, WAIT_BOUND};
    use tempfile::TempDir;

    const PROMPT: &str = "Harness for every Ticket run's sessions";
    const BOUND: Duration = Duration::from_secs(5);

    struct Fixture {
        harness: Child,
        root: TempDir,
        setup: Option<libc::pid_t>,
        keeper: Option<libc::pid_t>,
    }

    impl Fixture {
        fn start(mode: &str) -> Self {
            // SAFETY: adopt only this process's orphaned descendants. All
            // waits and cleanup below name an exact fixture-owned child.
            assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
            let root = TempDir::new().unwrap();
            let output = fs::File::create(root.path().join("output")).unwrap();
            let harness = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "linux::fixture_process",
                    "--ignored",
                    "--nocapture",
                ])
                .env("SETUP_LIFETIME_MODE", mode)
                .env("SETUP_LIFETIME_ROOT", root.path())
                // The supervisor owns this entire temporary subtree even if
                // SIGKILL prevents the fixture's Scenario from being dropped.
                .env("TMPDIR", root.path())
                .stdout(Stdio::from(output.try_clone().unwrap()))
                .stderr(Stdio::from(output))
                .spawn()
                .unwrap();
            let mut fixture = Self {
                harness,
                root,
                setup: None,
                keeper: None,
            };
            let deadline = Instant::now() + WAIT_BOUND;
            loop {
                if let Ok(pid) = fs::read_to_string(fixture.root.path().join("ready"))
                    && let Ok(pid) = pid.parse()
                {
                    fixture.setup = Some(pid);
                    fixture.keeper = fs::read_to_string(fixture.root.path().join("keeper"))
                        .ok()
                        .map(|pid| pid.parse().unwrap());
                    return fixture;
                }
                assert!(
                    fixture.harness.try_wait().unwrap().is_none(),
                    "fixture exited before setup reached its prompt: {}",
                    fixture.output()
                );
                assert!(Instant::now() < deadline, "setup never reached its prompt");
                thread::sleep(Duration::from_millis(10));
            }
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
                assert!(Instant::now() < deadline, "fixture hung: {}", self.output());
                thread::sleep(Duration::from_millis(10));
            }
            self.assert_setup_gone();
        }

        fn kill_harness(&mut self) {
            self.harness.kill().unwrap();
            assert_eq!(self.harness.wait().unwrap().signal(), Some(libc::SIGKILL));
            self.assert_setup_gone();
        }

        fn assert_setup_gone(&mut self) {
            let pid = self.setup.unwrap();
            let deadline = Instant::now() + BOUND;
            loop {
                let mut status = 0;
                // SAFETY: wait for only the recorded setup child, adopted
                // after its fixture harness exits, if it wasn't reaped there.
                match unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } {
                    found if found == pid => break,
                    -1 => {
                        let error = std::io::Error::last_os_error();
                        if error.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
                        // Already reaped by TerminalCommand in the fixture.
                        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
                        assert_eq!(
                            std::io::Error::last_os_error().raw_os_error(),
                            Some(libc::ESRCH)
                        );
                        break;
                    }
                    0 => {}
                    result => panic!("unexpected waitpid result: {result}"),
                }
                assert!(
                    Instant::now() < deadline,
                    "owned setup child {pid} survived its fixture: {}",
                    self.output()
                );
                thread::sleep(Duration::from_millis(10));
            }
            self.setup = None;
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.harness.kill();
            let _ = self.harness.wait();
            for pid in [self.setup, self.keeper].into_iter().flatten() {
                let mut status = 0;
                // Only kill when waitpid establishes it is still our live,
                // unreaped child; its PID cannot be reused in between.
                if unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } == 0 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                        libc::waitpid(pid, &mut status, 0);
                    }
                }
            }
        }
    }

    #[test]
    fn setup_ends_after_ordinary_fixture_completion() {
        Fixture::start("complete").finish(true);
    }

    #[test]
    fn setup_ends_after_fixture_assertion_unwinding() {
        let mut fixture = Fixture::start("assertion");
        fixture.finish(false);
        assert!(fixture.output().contains("intentional fixture assertion"));
    }

    #[test]
    fn setup_ends_after_fixture_timeout() {
        let mut fixture = Fixture::start("timeout");
        fixture.finish(false);
        assert!(
            fixture
                .output()
                .contains("the terminal command did not exit")
        );
    }

    #[test]
    fn setup_ends_after_sigkill_of_its_fixture() {
        let mut fixture = Fixture::start("sigkill");
        // End only this fixture harness; do not signal setup before checking.
        fixture.kill_harness();
    }

    #[test]
    fn setup_ends_after_sigkill_even_with_an_inherited_pty_master() {
        let mut fixture = Fixture::start("sigkill-inherited-master");
        fixture.kill_harness();
    }

    #[test]
    #[ignore = "subprocess entry point for setup lifetime tests"]
    fn fixture_process() {
        let mode = std::env::var("SETUP_LIFETIME_MODE").unwrap();
        let root = std::path::PathBuf::from(std::env::var_os("SETUP_LIFETIME_ROOT").unwrap());
        let scenario = Scenario::new();
        let inspect = |pid| {
            if mode == "sigkill-inherited-master" {
                // Model an inherited master that outlives the fixture. In
                // this multithreaded process, the fork child uses only
                // async-signal-safe syscalls, never Rust allocation/locks.
                let keeper = unsafe { libc::fork() };
                assert_ne!(keeper, -1);
                if keeper == 0 {
                    loop {
                        unsafe { libc::pause() };
                    }
                }
                fs::write(root.join("keeper"), keeper.to_string()).unwrap();
            }
            fs::write(root.join("ready.tmp"), format!("{pid}")).unwrap();
            fs::rename(root.join("ready.tmp"), root.join("ready")).unwrap();
            let deadline = Instant::now() + WAIT_BOUND;
            while !root.join("continue").exists() {
                assert!(
                    Instant::now() < deadline,
                    "supervisor never released fixture"
                );
                thread::sleep(Duration::from_millis(10));
            }
            assert_ne!(mode, "assertion", "intentional fixture assertion");
        };
        let mut steps = vec![TerminalStep::inspect(PROMPT, &inspect)];
        if mode == "complete" {
            steps.extend([
                TerminalStep::line("", "claude"),
                TerminalStep::line("Model for claude", ""),
                TerminalStep::line("Effort for claude", ""),
                TerminalStep::line("Self-merges?", ""),
                TerminalStep::line("Every Ticket run may start a Base fix", ""),
                TerminalStep::line("fast-forward", ""),
                TerminalStep::line("Security runs may fix", ""),
                TerminalStep::line("Ticket runs review their change", ""),
                TerminalStep::line("Run notifications, an email", "n"),
                TerminalStep::line("Run the Console?", "n"),
            ]);
        }
        let result = scenario.run_terminal_bounded(
            &["setup"],
            &[],
            &steps,
            if mode == "timeout" {
                Duration::from_secs(2)
            } else {
                WAIT_BOUND
            },
        );
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(result.terminal_restored);
        assert!(result.user_config.is_some());
    }
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "Linux-only parent-death guarantee with retained PTY master and PR_SET_CHILD_SUBREAPER reaping"]
fn setup_child_lifetime_after_fixture_death() {}
