//! Debug-only asset selection and confined disk reads.
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Default)]
pub(super) struct Development {
    directory: Option<PathBuf>,
}

impl Development {
    pub fn from_environment() -> anyhow::Result<Self> {
        #[cfg(debug_assertions)]
        if let Some(directory) = std::env::var_os("THIRDSHIFT_CONSOLE_ASSETS") {
            let directory = std::env::current_dir()?.join(directory);
            return Ok(Self {
                directory: Some(super::changes::observed_path(&directory)),
            });
        }
        Ok(Self::default())
    }

    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        // Check the raw spelling too: Path::components normalizes interior `.`.
        if path.is_empty()
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let directory = self.directory.as_ref().ok_or(io::ErrorKind::NotFound)?;
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(directory)?;
        let mut parts = Path::new(path).components().peekable();
        while let Some(part) = parts.next() {
            let Component::Normal(name) = part else {
                return Err(io::ErrorKind::InvalidInput.into());
            };
            let name = CString::new(name.as_encoded_bytes())?;
            let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
            if parts.peek().is_some() {
                flags |= libc::O_DIRECTORY;
            }
            // SAFETY: the directory fd and NUL-terminated component are live.
            // Each open pins the next component and refuses symlink races.
            let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openat returned a new, uniquely owned descriptor.
            file = unsafe { File::from_raw_fd(fd) };
        }
        if !file.metadata()?.is_file() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}
