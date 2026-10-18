//! Native process start identifiers; zombies have already died.

/// Pin the process before checking its start identity, so PID reuse cannot
/// redirect a Linux signal to an unrelated process.
#[cfg(target_os = "linux")]
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

#[cfg(target_os = "macos")]
pub(super) fn terminate(pid: u32, expected_start: &str) -> anyhow::Result<()> {
    let native_pid = i32::try_from(pid)?;
    anyhow::ensure!(native_pid > 0, "invalid Command process ID");
    // Apple's proc_info_private.h defines PROC_PIDUNIQIDENTIFIERINFO (17)
    // and this 56-byte layout. Read the PID version before rechecking start,
    // then let the kernel compare that version when delivering the signal.
    // https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info_private.h
    #[repr(C)]
    struct UniqueInfo {
        uuid: [u8; 16],
        unique_id: u64,
        parent_unique_id: u64,
        id_version: i32,
        parent_id_version: i32,
        reserved: [u64; 2],
    }
    let mut info = std::mem::MaybeUninit::<UniqueInfo>::uninit();
    let size = std::mem::size_of::<UniqueInfo>() as i32;
    // SAFETY: the buffer has the exact layout and size of the requested flavor.
    let read = unsafe { libc::proc_pidinfo(native_pid, 17, 0, info.as_mut_ptr().cast(), size) };
    anyhow::ensure!(
        read == size,
        "could not read Command process identity; no signal sent"
    );
    // SAFETY: proc_pidinfo initialized the complete structure.
    let info = unsafe { info.assume_init() };
    anyhow::ensure!(
        start(pid).as_deref() == Some(expected_start),
        "Command died or its process identity changed; no signal sent"
    );
    // libproc's identity-aware signal may be absent on older macOS versions.
    // Refuse there, rather than falling back to a PID-only kill with a race.
    // https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c
    let symbol =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
    anyhow::ensure!(
        !symbol.is_null(),
        "identity-checked Stop is unavailable on this macOS version; no signal sent"
    );
    // SAFETY: the symbol's ABI is int proc_signal_with_audittoken(audit_token_t*, int).
    let signal: unsafe extern "C" fn(*mut [u32; 8], i32) -> i32 =
        unsafe { std::mem::transmute(symbol) };
    let mut token = [0u32; 8];
    token[5] = pid;
    token[7] = info.id_version as u32;
    // SAFETY: the token is the eight-word audit_token_t layout; the kernel
    // identifies the target by PID and PID version, checking normal permissions.
    let error = unsafe { signal(&mut token, libc::SIGTERM) };
    if error != 0 {
        return Err(std::io::Error::from_raw_os_error(error).into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "macos")]
pub(super) fn start(pid: u32) -> Option<String> {
    let pid = i32::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: info has the exact size and alignment requested by PROC_PIDTBSDINFO.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    // SAFETY: proc_pidinfo returned a complete initialized proc_bsdinfo.
    let info = unsafe { info.assume_init() };
    if info.pbi_status == libc::SZOMB {
        return None;
    }
    Some(format!(
        "{}:{}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}
