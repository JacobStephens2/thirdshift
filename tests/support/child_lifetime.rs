//! Linux fixture children must not outlive the thread that starts them,
//! even when an inherited descriptor prevents EOF or terminal hangup.

use std::ffi::OsString;
use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{
    Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio,
};
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

/// Owns every process a `Scenario`'s Runs start, through success, assertion
/// unwinding and the death of the fixture harness. Each Run gets a guardian:
/// a Linux subreaper forked outside the harness's process group, which
/// starts thirdshift and adopts each descendant whose parent ends, even one
/// in a process group or session of its own. Dropping the owner, or the
/// harness dying for any reason, closes the lease every guardian polls; the
/// guardian then kills and reaps everything it owns.
pub struct OwnedRuns {
    /// Write end of the lease; only this process holds it outside an exec.
    lease: Option<OwnedFd>,
    lease_reader: OwnedFd,
    /// Write end each guardian inherits and holds until it exits, so drop
    /// can wait for cleanup to finish.
    done: Option<OwnedFd>,
    done_reader: File,
}

/// A thirdshift process started by [`OwnedRuns::spawn`]. Its guardian keeps
/// it unreaped until its status is read, as for a [`Child`], so its process
/// id stays its own until then.
pub struct OwnedRun {
    pid: u32,
    report: Option<UnixStream>,
    status: Option<ExitStatus>,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

impl OwnedRuns {
    pub fn new() -> Self {
        let (lease_reader, lease) = pipe();
        let (done_reader, done) = pipe();
        Self {
            lease: Some(lease),
            lease_reader,
            done: Some(done),
            done_reader: File::from(done_reader),
        }
    }

    /// Spawn `command` with its stdio as configured, under a new guardian.
    pub fn spawn(&self, command: &mut Command) -> OwnedRun {
        let (report, guardian_report) = UnixStream::pair().unwrap();
        let fds = [
            self.lease_reader.as_raw_fd(),
            guardian_report.as_raw_fd(),
            self.done.as_ref().unwrap().as_raw_fd(),
        ];
        // SAFETY: the post-fork closure calls only async-signal-safe
        // functions, and the processes that do not exec never return.
        unsafe { command.pre_exec(move || start_guarded(fds)) };
        let mut launcher = command.spawn().expect("could not start thirdshift");
        drop(guardian_report);
        // The launcher only forks the guardian, which outlives it.
        assert!(launcher.wait().unwrap().success());
        let mut run = OwnedRun {
            pid: 0,
            report: Some(report),
            status: None,
            stdin: launcher.stdin.take(),
            stdout: launcher.stdout.take(),
            stderr: launcher.stderr.take(),
        };
        run.pid = run
            .read_report()
            .flatten()
            .expect("the Run's guardian ended at once") as u32;
        run
    }
}

impl Drop for OwnedRuns {
    fn drop(&mut self) {
        drop(self.lease.take());
        drop(self.done.take());
        // EOF once every guardian has reaped what it owned and exited.
        let _ = std::io::Read::read_to_end(&mut self.done_reader, &mut Vec::new());
    }
}

impl OwnedRun {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&mut self) -> Option<ExitStatus> {
        if self.status.is_none() {
            let report = self.report.as_ref().unwrap();
            report.set_nonblocking(true).unwrap();
            let status = self.read_report();
            self.report
                .as_ref()
                .unwrap()
                .set_nonblocking(false)
                .unwrap();
            self.record(status.expect("the Run's guardian ended first"));
        }
        self.status
    }

    pub fn wait(&mut self) -> ExitStatus {
        if self.status.is_none() {
            let status = self.read_report();
            self.record(status.expect("the Run's guardian ended first"));
        }
        self.status.unwrap()
    }

    /// Like [`Child::wait_with_output`]: close stdin, read stdout and stderr
    /// until everything holding them lets go, then wait.
    pub fn wait_with_output(mut self) -> Output {
        drop(self.stdin.take());
        let stderr = self.stderr.take().map(|mut stderr| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut stderr, &mut bytes).unwrap();
                bytes
            })
        });
        let mut stdout = Vec::new();
        if let Some(mut out) = self.stdout.take() {
            std::io::Read::read_to_end(&mut out, &mut stdout).unwrap();
        }
        let stderr = stderr.map_or_else(Vec::new, |reader| reader.join().unwrap());
        Output {
            status: self.wait(),
            stdout,
            stderr,
        }
    }

    /// The next raw value the guardian reports, `None` at EOF, or
    /// `Some(None)` when nonblocking and none has arrived.
    fn read_report(&mut self) -> Option<Option<i32>> {
        let mut bytes = [0; 4];
        let mut read = 0;
        while read < bytes.len() {
            match std::io::Read::read(self.report.as_mut().unwrap(), &mut bytes[read..]) {
                Ok(0) => return None,
                Ok(n) => read += n,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && read == 0 => {
                    return Some(None);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("could not read the Run's guardian: {error}"),
            }
        }
        Some(Some(i32::from_ne_bytes(bytes)))
    }

    fn record(&mut self, status: Option<i32>) {
        if let Some(status) = status {
            self.status = Some(ExitStatus::from_raw(status));
            // Lets the guardian reap the Run, as waiting on a Child would,
            // and waits until it has.
            let mut report = self.report.take().unwrap();
            report.set_nonblocking(false).unwrap();
            report.shutdown(std::net::Shutdown::Write).unwrap();
            let _ = std::io::Read::read_to_end(&mut report, &mut Vec::new());
        }
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    // SAFETY: pipe2 writes two new descriptors that nothing else owns.
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    // SAFETY: as above, ownership passes to these OwnedFds.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

/// Runs in the forked launcher before exec. The launcher forks the guardian
/// and exits; the guardian forks the process that returns here to exec.
fn start_guarded([lease, report, done]: [libc::c_int; 3]) -> std::io::Result<()> {
    // SAFETY: fork, prctl and _exit are async-signal-safe.
    unsafe {
        match libc::fork() {
            -1 => return Err(std::io::Error::last_os_error()),
            0 => {}
            _ => libc::_exit(0),
        }
        if libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) == -1 {
            libc::_exit(1);
        }
        match libc::fork() {
            -1 => libc::_exit(1),
            0 => Ok(()),
            run => guard(run, lease, report, done),
        }
    }
}

/// The guardian's loop. It allocates nothing and takes no locks: it is a
/// fork of a multithreaded process that never execs.
unsafe fn guard(
    run: libc::pid_t,
    lease: libc::c_int,
    mut report: libc::c_int,
    done: libc::c_int,
) -> ! {
    unsafe {
        // Outside the harness's process group, a signal to that group or
        // terminal leaves the guardian to clean up after it.
        libc::setpgid(0, 0);
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::signal(signal, libc::SIG_IGN);
        }
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        for stdio in 0..3 {
            libc::dup2(null, stdio);
        }
        // Holding nothing else, the guardian keeps no pipe of the Run's or
        // of the harness's open, nor another Scenario's lease.
        let mut keep = [lease, report, done];
        keep.sort_unstable();
        let mut first = 3;
        for fd in keep {
            if fd > first {
                libc::close_range(first as libc::c_uint, (fd - 1) as libc::c_uint, 0);
            }
            first = fd + 1;
        }
        libc::close_range(first as libc::c_uint, libc::c_uint::MAX, 0);
        send(report, run);

        let mut reported = false;
        let mut released = false;
        loop {
            let mut owned = false;
            for_each_child(|child| {
                owned = true;
                if child != run {
                    libc::waitpid(child, std::ptr::null_mut(), libc::WNOHANG);
                    return;
                }
                if !reported && let Some(status) = exit_status(run) {
                    send(report, status);
                    reported = true;
                }
            });
            if reported && released && report != -1 {
                // Reaped before the harness hears EOF, so the Run's process
                // id is free once its wait returns, as with a Child.
                libc::waitpid(run, std::ptr::null_mut(), 0);
                libc::close(report);
                report = -1;
                continue;
            }
            if !owned {
                libc::_exit(0);
            }
            let mut fds = [
                libc::pollfd {
                    fd: lease,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: if released { -1 } else { report },
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            if libc::poll(fds.as_mut_ptr(), 2, 10) > 0 {
                if fds[0].revents != 0 {
                    break;
                }
                // The harness writes nothing; it shuts down its end once it
                // has read the Run's status, or drops it.
                released |= fds[1].revents != 0;
            }
        }
        loop {
            for_each_child(|child| {
                libc::kill(child, libc::SIGKILL);
            });
            match libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) {
                -1 if *libc::__errno_location() == libc::ECHILD => libc::_exit(0),
                0 | -1 => {
                    let pause = libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 10_000_000,
                    };
                    libc::nanosleep(&pause, std::ptr::null_mut());
                }
                _ => {}
            }
        }
    }
}

/// `run`'s raw wait status once it has exited, leaving it unreaped.
unsafe fn exit_status(run: libc::pid_t) -> Option<i32> {
    unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        if libc::waitid(libc::P_PID, run as libc::id_t, &mut info, flags) != 0
            || info.si_pid() != run
        {
            return None;
        }
        Some(match info.si_code {
            libc::CLD_EXITED => (info.si_status() & 0xff) << 8,
            libc::CLD_DUMPED => info.si_status() | 0x80,
            _ => info.si_status(),
        })
    }
}

unsafe fn send(report: libc::c_int, value: i32) {
    let bytes = value.to_ne_bytes();
    // The harness may have gone; MSG_NOSIGNAL turns SIGPIPE into an error.
    unsafe {
        libc::send(
            report,
            bytes.as_ptr().cast(),
            bytes.len(),
            libc::MSG_NOSIGNAL,
        )
    };
}

/// Call `f` with each child of this single-threaded process, zombies too,
/// reading `/proc` into a buffer on the stack.
unsafe fn for_each_child(mut f: impl FnMut(libc::pid_t)) {
    unsafe {
        let file = libc::open(
            c"/proc/thread-self/children".as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC,
        );
        if file == -1 {
            return;
        }
        let mut buffer = [0u8; 512];
        let mut pid: libc::pid_t = 0;
        loop {
            let read = libc::read(file, buffer.as_mut_ptr().cast(), buffer.len());
            if read <= 0 {
                break;
            }
            for &byte in &buffer[..read as usize] {
                if byte.is_ascii_digit() {
                    pid = pid * 10 + libc::pid_t::from(byte - b'0');
                } else if pid != 0 {
                    f(pid);
                    pid = 0;
                }
            }
        }
        if pid != 0 {
            f(pid);
        }
        libc::close(file);
    }
}
