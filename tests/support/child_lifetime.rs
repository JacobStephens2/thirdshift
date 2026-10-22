//! Linux fixture children must not outlive the thread that starts them,
//! even when an inherited descriptor prevents EOF or terminal hangup.

use std::os::unix::process::CommandExt;
use std::process::Command;

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
