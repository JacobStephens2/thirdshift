//! Linux fixture children must not outlive the thread that starts them,
//! even when an inherited descriptor prevents EOF or terminal hangup.

use std::ffi::OsString;
use std::fs::{self, File};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn kill_with_parent(command: &mut Command) {
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: the post-fork closure only makes async-signal-safe syscalls
    // and constructs errors from errno, without allocation or locking.
    unsafe {
        command.pre_exec(move || {
            // SIGKILL cannot be caught by setup's interruption handling.
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Parent death before prctl is not retroactively signalled.
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
            }
            Ok(())
        });
    }
}

/// Owns the command and its descendants through success, timeout and panic.
/// File-backed stdio avoids blocked input writes and output readers held
/// open by descendants. A separate subreaper also owns descendants that
/// create new process groups, as release-summary Harnesses do.
pub struct FixtureChild {
    guardian: Child,
    root: tempfile::TempDir,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Invocation {
    program: OsString,
    args: Vec<OsString>,
}

impl FixtureChild {
    /// Release fixtures configure arguments, cwd and environment here; stdio
    /// and child lifetime belong to this helper. The guardian inherits the
    /// fixture environment with the command's explicit overrides/removals.
    pub fn spawn(command: &mut Command, input: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let invocation = Invocation {
            program: command.get_program().to_owned(),
            args: command.get_args().map(OsString::from).collect(),
        };
        fs::write(
            root.path().join("command"),
            serde_json::to_vec(&invocation).unwrap(),
        )
        .unwrap();
        fs::write(root.path().join("stdin"), input).unwrap();
        for stream in ["stdout", "stderr", "guardian-stderr"] {
            File::create(root.path().join(stream)).unwrap();
        }
        let mut guardian = Command::new(std::env::current_exe().unwrap());
        guardian.args([
            "--exact",
            "support::child_lifetime::fixture_guardian",
            "--ignored",
            "--nocapture",
        ]);
        if let Some(dir) = command.get_current_dir() {
            guardian.current_dir(dir);
        }
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    guardian.env(key, value);
                }
                None => {
                    guardian.env_remove(key);
                }
            }
        }
        let guardian = guardian
            .env("THIRDSHIFT_FIXTURE_COMMAND", root.path())
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(File::create(root.path().join("guardian-stderr")).unwrap())
            .spawn()
            .expect("could not start fixture guardian");
        Self { guardian, root }
    }

    pub fn wait(mut self, bound: Duration) -> Output {
        let deadline = Instant::now() + bound;
        loop {
            if let Some(status) = self.guardian.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "fixture guardian failed: {}",
                    self.diagnostics()
                );
                let status = fs::read_to_string(self.root.path().join("status"))
                    .unwrap()
                    .parse()
                    .unwrap();
                return Output {
                    status: ExitStatus::from_raw(status),
                    stdout: self.read("stdout"),
                    stderr: self.read("stderr"),
                };
            }
            if Instant::now() >= deadline {
                self.stop();
                panic!("release fixture exceeded {bound:?}\n{}", self.diagnostics());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn read(&self, stream: &str) -> Vec<u8> {
        fs::read(self.root.path().join(stream)).unwrap()
    }

    fn diagnostics(&self) -> String {
        format!(
            "stdout:\n{}\nstderr:\n{}\nguardian stderr:\n{}",
            String::from_utf8_lossy(&self.read("stdout")),
            String::from_utf8_lossy(&self.read("stderr")),
            String::from_utf8_lossy(&self.read("guardian-stderr"))
        )
    }

    fn stop(&mut self) {
        // Closing the only writer requests cleanup. SIGKILL of this process
        // closes it too; exec cannot leak its close-on-exec writer to children.
        drop(self.guardian.stdin.take());
        let _ = self.guardian.wait();
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Runs in a dedicated process containing only this fixture's descendants.
/// Killing direct children orphans the next generation to the subreaper, so
/// cleanup reaches new sessions/process groups without a global PID search.
struct Descendants;

impl Drop for Descendants {
    fn drop(&mut self) {
        loop {
            // Linux may adopt an orphan onto the guardian's main thread,
            // rather than the libtest thread that spawned the command.
            for task in fs::read_dir("/proc/self/task").unwrap() {
                let children = fs::read_to_string(task.unwrap().path().join("children")).unwrap();
                for pid in children.split_whitespace() {
                    let pid: libc::pid_t = pid.parse().unwrap();
                    // SAFETY: these are this process's unreaped children;
                    // their PIDs cannot be reused before waitpid reaps them.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            }
            let mut status = 0;
            match unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) } {
                -1 => {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        return;
                    }
                    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
                }
                0 => thread::sleep(Duration::from_millis(10)),
                _ => {}
            }
        }
    }
}

#[test]
#[ignore = "subprocess entry point for owned fixture commands"]
fn fixture_guardian() {
    let root = std::path::PathBuf::from(std::env::var_os("THIRDSHIFT_FIXTURE_COMMAND").unwrap());
    // SAFETY: this guardian has no unrelated children. Only this process's
    // descendants can be adopted; the main test process is unaffected.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
    let descendants = Descendants;
    let invocation: Invocation =
        serde_json::from_slice(&fs::read(root.join("command")).unwrap()).unwrap();
    let mut command = Command::new(invocation.program);
    command
        .args(invocation.args)
        .stdin(File::open(root.join("stdin")).unwrap())
        .stdout(File::create(root.join("stdout")).unwrap())
        .stderr(File::create(root.join("stderr")).unwrap());
    kill_with_parent(&mut command);
    let mut child = command.spawn().unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        let mut lease = libc::pollfd {
            fd: 0,
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        // SAFETY: poll inspects only the guardian's private stdin lease.
        let polled = unsafe { libc::poll(&mut lease, 1, 10) };
        if polled > 0 {
            break None;
        }
        if polled == -1 {
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::Interrupted
            );
        }
    };
    drop(descendants);
    if let Some(status) = status {
        fs::write(root.join("status"), status.into_raw().to_string()).unwrap();
    }
}
