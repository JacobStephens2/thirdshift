//! Opaque review reports, named in the prompt and kept beside the Session log.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};
use tempfile::TempDir;

use crate::{git::Git, prompt, skills};

/// One session's report directory, shared with its Resume so the latest
/// reports are kept with that session's log too. Never shared across reviews.
pub(super) struct ReviewReports(TempDir);

impl ReviewReports {
    pub fn new(worktree: &Path) -> Result<Self> {
        skills::exclude(&Git::new(worktree), "/.thirdshift-review-*/")?;
        let directory = tempfile::Builder::new()
            .prefix(".thirdshift-review-")
            .tempdir_in(worktree)
            .context("could not create the review reports directory")?;
        Ok(Self(directory))
    }

    pub fn prompt(&self, text: &str) -> String {
        text.replace(
            prompt::REVIEW_REPORTS_DIRECTORY,
            &self.0.path().to_string_lossy(),
        )
    }

    /// Copy whichever reports exist without reading or interpreting them.
    /// Missing reports and failures to keep them are progress lines, never
    /// changes to the session's outcome.
    pub fn keep(&self, log: &Path) -> Vec<String> {
        let mut lines = Vec::new();
        let mut found = false;
        for axis in ["standards", "spec"] {
            let report = self.0.path().join(format!("{axis}.md"));
            match fs::metadata(&report) {
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    lines.push(format!("could not find {axis} report: {error}"));
                    continue;
                }
                Ok(_) => found = true,
            }
            let kept = log.with_extension(axis);
            if let Err(error) = fs::copy(&report, &kept) {
                lines.push(format!("could not keep {}: {error}", kept.display()));
            }
        }
        if !found {
            lines.push("no review reports were left; carrying on".to_string());
        }
        lines
    }
}
