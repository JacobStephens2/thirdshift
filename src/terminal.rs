//! Shared terminal input for Setup and Console passwords.
use crate::interrupt;
use anyhow::{Context, Result};

/// Read without read-ahead, so pasted answers remain for the next question.
/// Polling lets recorded signals end input even when no next line arrives.
pub fn line() -> Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        interrupt::check()?;
        let mut stdin = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: stdin points to one valid pollfd; polling does not own fd 0.
        let ready = unsafe { libc::poll(&mut stdin, 1, 100) };
        interrupt::check()?;
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if ready == 0 {
            continue;
        }
        if stdin.revents & libc::POLLNVAL != 0 {
            return Err(std::io::Error::from_raw_os_error(libc::EBADF).into());
        }
        let mut byte = 0u8;
        // SAFETY: byte is writable for the one byte requested from fd 0.
        let read = unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) };
        interrupt::check()?;
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if read == 0 {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        line.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    let line = String::from_utf8(line).context("terminal input is not UTF-8")?;
    interrupt::check()?;
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_owned()))
}

/// Restore the exact saved terminal settings on every return path.
pub struct EchoOff {
    saved: libc::termios,
}

impl EchoOff {
    pub fn new() -> Result<Self> {
        // SAFETY: tcgetattr fills this termios from the open stdin descriptor.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("can't read the terminal's settings");
        }
        let echo_off = Self { saved };
        let mut hidden = saved;
        hidden.c_lflag &= !libc::ECHO;
        // SAFETY: fd 0 is open, and hidden is a valid termios.
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &hidden) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("can't turn the terminal's echo off");
        }
        Ok(echo_off)
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: fd 0 is open, and saved was read from it.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.saved) };
    }
}
