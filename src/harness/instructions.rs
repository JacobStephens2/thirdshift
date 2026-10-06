//! Instruction-file fallback links shared by Harness adapters. Existing
//! files, including dangling links, always win; only root CLAUDE.md falls back.
use crate::{git::Git, skills};
use anyhow::{Context, Result};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::symlink;
use std::path::Path;

/// Link `name` to root CLAUDE.md only when none of `name` and `preferred`
/// already exists. Exclude it before creating it, using the repository's
/// shared, locked exclude file.
pub(super) fn link_fallback(worktree: &Path, name: &str, preferred: &[&str]) -> Result<()> {
    let exists = |name: &str| -> Result<bool> {
        match fs::symlink_metadata(worktree.join(name)) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error)
                .with_context(|| format!("could not inspect {}", worktree.join(name).display())),
        }
    };
    if exists(name)? {
        return Ok(());
    }
    for name in preferred {
        if exists(name)? {
            return Ok(());
        }
    }
    if !worktree.join("CLAUDE.md").is_file() {
        return Ok(());
    }
    skills::exclude(&Git::new(worktree), &format!("/{name}"))?;
    symlink("CLAUDE.md", worktree.join(name)).with_context(|| {
        format!(
            "could not link {} to CLAUDE.md",
            worktree.join(name).display()
        )
    })
}
