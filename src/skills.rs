//! The Factory skills, embedded at compile time (ADR-0001), written out once
//! per Command and linked into each worktree its sessions run in, where the
//! Harness finds project skills (ADR-0012).

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result, bail};
use include_dir::{Dir, include_dir};
use tempfile::TempDir;

use crate::git::Git;
use crate::harness::Adapter;

mod directory;
use directory::CheckedDirectory;

#[cfg(test)]
mod tests;

/// The Factory skills, as they are written out and the Prompts and skills
/// page shows them, each in its `thirdshift-<skill>` directory.
pub static SKILLS: Dir = include_dir!("$CARGO_MANIFEST_DIR/skills");

/// A skill resolved by name, with the instructions the session must read.
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
}

enum Source {
    Factory,
    Repository(PathBuf),
}

/// Resolve exact names across Factory and repository skills before trying
/// a Factory shorthand. Only the committed worktree is searched.
pub fn find(worktree: &Path, name: &str) -> Result<Skill> {
    let mut available = BTreeMap::new();
    for dir in SKILLS.dirs() {
        available.insert(dir.path().to_string_lossy().into_owned(), Source::Factory);
    }
    for folder in [".agents/skills", ".claude/skills"] {
        let entries = match fs::read_dir(worktree.join(folder)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("could not list {folder}")),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path().join("SKILL.md");
            if path.is_file() {
                let name = entry.file_name().into_string().map_err(|_| {
                    anyhow::anyhow!(
                        "skill directory name is not UTF-8: {}",
                        entry.path().display()
                    )
                })?;
                available.entry(name).or_insert(Source::Repository(path));
            }
        }
    }
    let selected = available.get_key_value(name).or_else(|| {
        available
            .get_key_value(&format!("thirdshift-{name}"))
            .filter(|(_, source)| matches!(source, Source::Factory))
    });
    if let Some((name, source)) = selected {
        return Ok(Skill {
            path: match source {
                Source::Factory => written_out()?.join(name).join("SKILL.md"),
                Source::Repository(path) => path.clone(),
            },
            name: name.clone(),
        });
    }
    bail!(
        "unknown skill {name:?}; available skills: {}",
        available.keys().cloned().collect::<Vec<_>>().join(", ")
    )
}

/// The temp directory the Command wrote the Factory skills out to, once it
/// has.
static WRITTEN: Mutex<Option<TempDir>> = Mutex::new(None);

/// Link every Factory skill into `worktree`'s project skills for `adapter`,
/// `.claude/skills/` or `.agents/skills/`, writing them out first if the
/// Command has not yet, and make sure the repository's `.git/info/exclude`
/// keeps them out of git. The links are left in place: they go with the
/// worktree.
pub fn link_into(worktree: &Path, adapter: &dyn Adapter) -> Result<()> {
    let dir = adapter.project_skills();
    // Refuse an unsafe path before writing skills or updating excludes.
    let project_skills = CheckedDirectory::check(worktree, Path::new(dir))?;
    let written = written_out()?;
    exclude(&Git::new(worktree), &format!("/{dir}/thirdshift-*"))?;
    let project_skills = project_skills.create_missing()?;
    for skill in SKILLS.dirs() {
        let name = skill.path();
        project_skills.link(&written.join(name), name)?;
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
