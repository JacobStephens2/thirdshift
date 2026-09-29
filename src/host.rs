//! This machine, as the OS names it.

/// The hostname, or `None` if the OS won't say.
pub fn name() -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the pointer and length describe `buffer`, which outlives the call.
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return None;
    }
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    Some(String::from_utf8_lossy(&buffer[..end]).into_owned()).filter(|name| !name.is_empty())
}
