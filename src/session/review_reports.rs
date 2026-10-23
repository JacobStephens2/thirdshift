//! Opaque review reports and summary, kept beside the Session trace.

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
        for (name, file, extension) in [
            ("standards", "standards.md", "standards"),
            ("spec", "spec.md", "spec"),
            ("summary", "summary.json", "review-summary.json"),
        ] {
            let report = self.0.path().join(file);
            match fs::metadata(&report) {
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    lines.push(format!("could not find {name} report: {error}"));
                    continue;
                }
                Ok(_) => found = true,
            }
            let kept = crate::logs::session::original(log).with_extension(extension);
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
