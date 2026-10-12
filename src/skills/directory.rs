//! Directory handles keep validation and Factory skill writes on the same
//! directories. Joined paths below are only used in error messages.

use std::ffi::{CString, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(super) struct CheckedDirectory {
    handle: File,
    path: PathBuf,
    missing: Vec<OsString>,
}

impl CheckedDirectory {
    /// Open every existing component without following symlinks. Remember
    /// the missing suffix, but create nothing until the whole check passes.
    pub(super) fn check(worktree: &Path, relative: &Path) -> Result<Self> {
        let mut names = Vec::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                bail!(
                    "project skills directory {} is not worktree-relative",
                    relative.display()
                );
            };
            names.push(name.to_owned());
        }
        let handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(worktree)
            .with_context(|| format!("refusing project skills directory {}", worktree.display()))?;
        let mut checked = Self {
            handle,
            path: worktree.to_owned(),
            missing: Vec::new(),
        };
        for (at, name) in names.iter().enumerate() {
            let path = checked.path.join(name);
            match open_child(&checked.handle, Path::new(name)) {
                Ok(handle) => {
                    checked.handle = handle;
                    checked.path = path;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    checked.missing = names[at..].to_vec();
                    break;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("refusing project skills directory {}", path.display())
                    });
                }
            }
        }
        Ok(checked)
    }

    /// Create the missing suffix relative to its checked parent, opening
    /// each child with the same no-symlink rule before using it as a parent.
    pub(super) fn create_missing(mut self) -> Result<Self> {
        for name in std::mem::take(&mut self.missing) {
            let path = self.path.join(&name);
            let name = CString::new(name.as_bytes())?;
            // SAFETY: the live directory handle and NUL-terminated name
            // remain valid for this call; mkdirat retains neither pointer.
            if unsafe { libc::mkdirat(self.handle.as_raw_fd(), name.as_ptr(), 0o777) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error)
                        .with_context(|| format!("could not create {}", path.display()));
                }
            }
            self.handle = open_child(
                &self.handle,
                Path::new(std::ffi::OsStr::from_bytes(name.as_bytes())),
            )
            .with_context(|| format!("refusing project skills directory {}", path.display()))?;
            self.path = path;
        }
        Ok(self)
    }

    /// Remove an entry and link its replacement using this directory's
    /// handle for both operations, never resolving its joined path again.
    pub(super) fn link(&self, target: &Path, name: &Path) -> Result<()> {
        let path = self.path.join(name);
        let name = CString::new(name.as_os_str().as_bytes())?;
        let target = CString::new(target.as_os_str().as_bytes())?;
        // SAFETY: the live handle and NUL-terminated name are valid. Zero
        // flags remove only the named entry, without following a symlink.
        if unsafe { libc::unlinkat(self.handle.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error).with_context(|| format!("could not replace {}", path.display()));
            }
        }
        // SAFETY: both NUL-terminated strings and the directory handle
        // remain valid; symlinkat retains none of these pointers.
        if unsafe { libc::symlinkat(target.as_ptr(), self.handle.as_raw_fd(), name.as_ptr()) } != 0
        {
            return Err(io::Error::last_os_error())
                .with_context(|| format!("could not link {}", path.display()));
        }
        Ok(())
    }
}

fn open_child(parent: &File, name: &Path) -> io::Result<File> {
    let name = CString::new(name.as_os_str().as_bytes())?;
    // SAFETY: parent is live and name is NUL-terminated for the call. The
    // returned descriptor belongs only to us and is immediately owned.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat succeeded and gave us a new, exclusively owned fd.
    Ok(unsafe { File::from_raw_fd(fd) })
}
