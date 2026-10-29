//! Fallback cleanup for fixtures whose CLI owns a process group.

pub(crate) fn kill_process_and_group(pid: libc::pid_t) {
    if pid > 0 {
        // SAFETY: kill has no memory-safety preconditions; the positive PID
        // records the fixture's own process group.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
        let _ = super::signal(pid, libc::SIGKILL);
    }
}
