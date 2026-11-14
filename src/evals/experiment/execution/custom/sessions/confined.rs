//! Pin the runner output directory and open only regular top-level files.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::{Context, Result, ensure};

pub(super) struct Directory(File);

impl Directory {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(path)
                .context("cannot open Custom runner output directory")?,
        ))
    }

    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        ensure!(
            !name.is_empty()
                && name != "."
                && name != ".."
                && !name.contains(['/', '\\'])
                && !name.chars().any(char::is_control),
            "Custom runner Session file must be a top-level filename"
        );
        let name = CString::new(name)?;
        // The directory descriptor pins ownership through renames. NOFOLLOW
        // rejects symlinks atomically; NONBLOCK avoids waiting on a FIFO.
        let descriptor = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error).context("cannot open Custom runner Session file");
        }
        // openat returned this newly owned descriptor.
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        ensure!(
            file.metadata()?.is_file(),
            "Custom runner Session file must be a regular file"
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }
}
