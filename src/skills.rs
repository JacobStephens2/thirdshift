//! The Factory skills, embedded at compile time (ADR-0001), written out once
//! per Command and linked into each worktree its sessions run in, where the
//! Harness finds project skills (ADR-0012).

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result};
use include_dir::{Dir, include_dir};
use tempfile::TempDir;

use crate::git::Git;
use crate::harness::Adapter;

/// The Factory skills, as they are written out and the Prompts and skills
/// page shows them, each in its `thirdshift-<skill>` directory.
pub static SKILLS: Dir = include_dir!("$CARGO_MANIFEST_DIR/skills");

/// The temp directory the Command wrote the Factory skills out to, once it
/// has.
static WRITTEN: Mutex<Option<TempDir>> = Mutex::new(None);

/// Link every Factory skill into `worktree`'s project skills for `adapter`,
/// `.claude/skills/` or `.agents/skills/`, writing them out first if the
/// Command has not yet, and make sure the repository's `.git/info/exclude`
/// keeps them out of git. The links are left in place: they go with the
/// worktree.
pub fn link_into(worktree: &Path, adapter: &dyn Adapter) -> Result<()> {
    let written = written_out()?;
    let dir = adapter.project_skills();
    exclude(&Git::new(worktree), &format!("/{dir}/thirdshift-*"))?;
    let project_skills = worktree.join(dir);
    fs::create_dir_all(&project_skills)
        .with_context(|| format!("could not create {}", project_skills.display()))?;
    for skill in SKILLS.dirs() {
        let name = skill.path();
        let link = project_skills.join(name);
        if fs::symlink_metadata(&link).is_ok() {
            fs::remove_file(&link)
                .with_context(|| format!("could not replace {}", link.display()))?;
        }
        symlink(written.join(name), &link)
            .with_context(|| format!("could not link {}", link.display()))?;
    }
    Ok(())
}

/// Remove the Factory skills the Command wrote out, if it did, once its
/// sessions are over.
pub fn remove_written() {
    drop(
        WRITTEN
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    );
}

/// Where the Command wrote the Factory skills out, writing them now if it
/// has not yet.
pub(crate) fn written_out() -> Result<PathBuf> {
    let mut written = WRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(dir) = &*written {
        return Ok(dir.path().to_owned());
    }
    let dir = tempfile::Builder::new()
        .prefix("thirdshift-skills-")
        .tempdir()?;
    SKILLS
        .extract(dir.path())
        .context("could not write the Factory skills")?;
    let path = dir.path().to_owned();
    *written = Some(dir);
    Ok(path)
}

/// Add the pattern `exclude` to the `.git/info/exclude` of the repository
/// `git` works in, unless it is there already. The file is shared by every
/// worktree, so the entry is left there, under a lock other Runs from the
/// same Launch directory take too.
pub(crate) fn exclude(git: &Git, exclude: &str) -> Result<()> {
    let _lock = git.lock("thirdshift-exclude.lock")?;
    let info = git.common_dir()?.join("info");
    let path = info.join("exclude");
    let existing = match fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    if existing.lines().any(|line| line == exclude) {
        return Ok(());
    }
    fs::create_dir_all(&info).with_context(|| format!("could not create {}", info.display()))?;
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{separator}{exclude}"))
        .with_context(|| format!("could not add an exclude pattern to {}", path.display()))
}
