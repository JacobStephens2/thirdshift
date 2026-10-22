//! Linux fixture children must not outlive the thread that starts them,
//! even when an inherited descriptor prevents EOF or terminal hangup.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
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

/// A process group held alive by a guardian until the fixture closes its
/// private pipe, including when SIGKILL closes the fixture's descriptors.
/// The guardian pins the group ID, so cleanup never signals a reused PID.
struct Group(Option<Child>);

impl Group {
    fn start() -> Self {
        Self(Some(
            Command::new("bash")
                // No `set -e`: EOF must reach kill even though read fails.
                .args(["-c", "read -r; kill -KILL -- -$$"])
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("could not start fixture guardian"),
        ))
    }

    fn stop(&mut self) {
        if let Some(mut guardian) = self.0.take() {
            // SAFETY: this still-unreaped child owns the group ID. The group
            // contains only the guardian and commands this fixture started.
            unsafe { libc::kill(-(guardian.id() as libc::pid_t), libc::SIGKILL) };
            let _ = guardian.wait();
        }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Owns the command and its descendants through success, timeout and panic.
/// File-backed stdio avoids blocked input writes and output readers held
/// open by descendants; the deadline covers the whole wait.
pub struct FixtureChild {
    child: Child,
    group: Group,
    stdout: File,
    stderr: File,
}

impl FixtureChild {
    pub fn spawn(command: &mut Command, input: &str) -> Self {
        let group = Group::start();
        let mut stdin = tempfile::tempfile().unwrap();
        stdin.write_all(input.as_bytes()).unwrap();
        stdin.rewind().unwrap();
        let stdout = tempfile::tempfile().unwrap();
        let stderr = tempfile::tempfile().unwrap();
        kill_with_parent(command);
        let child = command
            .process_group(group.0.as_ref().unwrap().id() as libc::pid_t)
            .stdin(stdin)
            .stdout(stdout.try_clone().unwrap())
            .stderr(stderr.try_clone().unwrap())
            .spawn()
            .expect("could not start fixture command");
        Self {
            child,
            group,
            stdout,
            stderr,
        }
    }

    pub fn wait(mut self, bound: Duration) -> Output {
        let deadline = Instant::now() + bound;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                self.group.stop();
                let _ = self.child.wait();
                panic!(
                    "release fixture exceeded {bound:?}\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&Self::read(&mut self.stdout)),
                    String::from_utf8_lossy(&Self::read(&mut self.stderr)),
                );
            }
            thread::sleep(Duration::from_millis(10));
        };
        // Descendants must end even when the immediate command succeeded.
        self.group.stop();
        Output {
            status,
            stdout: Self::read(&mut self.stdout),
            stderr: Self::read(&mut self.stderr),
        }
    }

    fn read(file: &mut File) -> Vec<u8> {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }
}

impl Drop for FixtureChild {
    fn drop(&mut self) {
        self.group.stop();
        let _ = self.child.wait();
    }
}
