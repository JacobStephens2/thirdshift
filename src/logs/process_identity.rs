//! Native process start identifiers; zombies have already died.

/// Pin the process before checking its start identity, so PID reuse cannot
/// redirect a Linux signal to an unrelated process.
pub(super) fn terminate(pid: u32, expected_start: &str) -> anyhow::Result<()> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: pidfd_open takes a PID and flags, and returns an owned descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: a successful pidfd_open returned a new descriptor owned here.
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    anyhow::ensure!(
        start(pid).as_deref() == Some(expected_start),
        "Command died or its process identity changed; no signal sent"
    );
    // SAFETY: the descriptor pins the verified process; a null siginfo and
    // zero flags request an ordinary SIGTERM, never a process-group signal.
    let sent = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if sent < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) fn start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm is parenthesized and can itself contain spaces and parentheses.
    let (_, fields) = stat.rsplit_once(") ")?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    if matches!(*fields.first()?, "Z" | "X") {
        return None;
    }
    let ticks = fields.get(19)?; // field 22; fields here start at field 3
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    Some(format!("{}:{ticks}", boot.trim()))
}
