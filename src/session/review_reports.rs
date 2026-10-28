//! Opaque review reports and summary, kept beside the Session trace.

use std::cell::OnceCell;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
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
    /// A report is kept only when it is a regular file. A symlink or any
    /// other non-regular file is refused in a progress line. A missing
    /// report stays skipped. None of these change the session's outcome.
    pub fn keep(&self, log: &Path) -> Vec<String> {
        let mut lines = Vec::new();
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
            let report = self.directory.path().join(file);
            match fs::symlink_metadata(&report) {
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    lines.push(format!("could not find {name} report: {error}"));
                    continue;
                }
                Ok(metadata) if metadata.is_symlink() => {
                    found = true;
                    lines.push(format!("{name} report is a symlink; not keeping it"));
                    continue;
                }
                Ok(metadata) if !metadata.is_file() => {
                    found = true;
                    lines.push(format!(
                        "{name} report is not a regular file; not keeping it"
                    ));
                    continue;
                }
                Ok(_) => found = true,
            }
            if let Err(error) = copy_regular_file(&report, &kept) {
                lines.push(format!("could not keep {}: {error}", kept.display()));
            }
        }
        if !found {
            lines.push("no review reports were left; carrying on".to_string());
        }
        lines
    }
}

/// Copy `report` onto `kept` without following a symlink at either path.
/// The session has already exited, so following a symlink would read or
/// write a host path outside the report directory.
fn copy_regular_file(report: &Path, kept: &Path) -> Result<()> {
    let mut source = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(report)
    {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            bail!("{} is a symlink", report.display())
        }
        Err(error) => return Err(error.into()),
    };
    let pinned = source.metadata()?;
    let current = fs::symlink_metadata(report)?;
    if !pinned.is_file()
        || !current.is_file()
        || current.dev() != pinned.dev()
        || current.ino() != pinned.ino()
    {
        bail!("{} is not a regular file", report.display());
    }
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
    io::copy(&mut source, &mut destination)?;
    destination.set_permissions(pinned.permissions())?;
    Ok(())
}
