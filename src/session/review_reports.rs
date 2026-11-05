//! Opaque review reports and summary, kept beside the Session trace.

use std::cell::OnceCell;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

use crate::{git::Git, prompt, skills};

/// One session's report directory, shared with its Resume so the latest
/// reports are kept with that session's log too. Never shared across reviews.
pub(super) struct ReviewReports {
    directory: TempDir,
    summary_log: OnceCell<PathBuf>,
}

impl ReviewReports {
    pub fn new(worktree: &Path) -> Result<Self> {
        skills::exclude(&Git::new(worktree), "/.thirdshift-review-*/")?;
        let directory = tempfile::Builder::new()
            .prefix(".thirdshift-review-")
            .tempdir_in(worktree)
            .context("could not create the review reports directory")?;
        Ok(Self {
            directory,
            summary_log: OnceCell::new(),
        })
    }

    pub fn prompt(&self, text: &str) -> String {
        text.replace(
            prompt::REVIEW_REPORTS_DIRECTORY,
            &self.directory.path().to_string_lossy(),
        )
    }

    /// Copy whichever reports exist without reading or interpreting them.
    /// The report directory is opened with `O_DIRECTORY` and `O_NOFOLLOW`,
    /// and each report is copied from that directory. A symlink directory
    /// is refused. A report is kept only when it is a regular file. A
    /// symlink or any other non-regular file is refused in a progress line.
    /// A missing report stays skipped. None of these change the session's
    /// outcome.
    pub fn keep(&self, log: &Path) -> Vec<String> {
        let mut lines = Vec::new();
        let directory = match open_report_directory(self.directory.path()) {
            Ok(directory) => directory,
            Err(error) if directory_is_symlink(self.directory.path(), &error) => {
                lines.push(
                    "review reports directory is a symlink; not keeping its reports".to_string(),
                );
                return lines;
            }
            Err(error) => {
                lines.push(format!("could not open review reports directory: {error}"));
                return lines;
            }
        };
        let mut found = false;
        let original = crate::logs::session::original(log);
        // A Resume continues this review. Keep its latest summary on the
        // originating Session so the Run counts that review only once.
        let summary = self
            .summary_log
            .get_or_init(|| original.with_extension("review-summary.json"));
        for (name, file, kept) in [
            (
                "standards",
                "standards.md",
                original.with_extension("standards"),
            ),
            ("spec", "spec.md", original.with_extension("spec")),
            ("summary", "summary.json", summary.clone()),
        ] {
            let mut source = match open_report(&directory, file) {
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
                    found = true;
                    lines.push(format!("{name} report is a symlink; not keeping it"));
                    continue;
                }
                Err(error) => {
                    lines.push(format!("could not find {name} report: {error}"));
                    continue;
                }
                Ok(source) => source,
            };
            let metadata = match source.metadata() {
                Ok(metadata) if metadata.is_file() => metadata,
                Ok(_) => {
                    found = true;
                    lines.push(format!(
                        "{name} report is not a regular file; not keeping it"
                    ));
                    continue;
                }
                Err(error) => {
                    found = true;
                    lines.push(format!("could not find {name} report: {error}"));
                    continue;
                }
            };
            found = true;
            if let Err(error) = copy_regular_file(&mut source, &metadata, &kept) {
                lines.push(format!("could not keep {}: {error}", kept.display()));
            }
        }
        if !found {
            lines.push("no review reports were left; carrying on".to_string());
        }
        lines
    }
}

/// Open the report directory without following a symlink planted at its path.
/// Later reads use this handle, so replacing the path cannot redirect them.
fn open_report_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}

/// `open` already refused this path. Linux reports an unfollowed symlink
/// opened with `O_DIRECTORY` as `ENOTDIR`; other kernels report `ELOOP`.
/// `lstat` only names that refusal. It is not used to open the directory.
fn directory_is_symlink(path: &Path, error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
        || (error.raw_os_error() == Some(libc::ENOTDIR)
            && std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()))
}

/// Open `file` in `directory`. `O_NOFOLLOW` refuses a symlink at that name.
fn open_report(directory: &File, file: &str) -> io::Result<File> {
    let name =
        CString::new(file).map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    // SAFETY: `directory` is live and `name` is NUL-terminated for this call.
    // openat retains neither. A non-negative fd is a new descriptor we own.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat succeeded and returned a new, uniquely owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Copy the already-opened report onto `kept` without following a symlink
/// at the destination. The session has already exited, so following a
/// symlink would write a host path outside the Session trace's directory.
fn copy_regular_file(source: &mut File, metadata: &std::fs::Metadata, kept: &Path) -> Result<()> {
    let mut destination = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(kept)
    {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            bail!("{} is a symlink", kept.display())
        }
        Err(error) => return Err(error.into()),
    };
    io::copy(source, &mut destination)?;
    destination.set_permissions(metadata.permissions())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;

    #[test]
    fn a_symlink_review_directory_is_not_kept_beside_the_session_trace() {
        let worktree = crate::test_support::scratch_worktree();
        let reports = ReviewReports::new(worktree.path()).unwrap();
        let directory = reports.directory.path().to_path_buf();
        let planted = worktree.path().join("planted");
        fs::create_dir(&planted).unwrap();
        for file in ["standards.md", "spec.md", "summary.json"] {
            fs::write(planted.join(file), format!("host secret {file}\n")).unwrap();
        }
        fs::remove_dir_all(&directory).unwrap();
        symlink(&planted, &directory).unwrap();

        let log = worktree.path().join("7-stamp-implement.logs.jsonl");
        let lines = reports.keep(&log);
        let original = crate::logs::session::original(&log);
        for extension in ["standards", "spec", "review-summary.json"] {
            assert!(
                fs::symlink_metadata(original.with_extension(extension)).is_err(),
                "kept {extension} from the symlink directory"
            );
        }
        let rendered = lines.join("\n");
        assert!(
            rendered.contains("review reports directory is a symlink; not keeping its reports"),
            "{rendered}"
        );
        assert!(
            !rendered.contains("no review reports were left"),
            "{rendered}"
        );
        for file in ["standards.md", "spec.md", "summary.json"] {
            assert_eq!(
                fs::read_to_string(planted.join(file)).unwrap(),
                format!("host secret {file}\n")
            );
        }
    }
}
