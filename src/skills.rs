//! The Factory skills, embedded at compile time (ADR-0001), written out once
//! per Command and linked into each worktree its sessions run in, where the
//! Harness finds project skills (ADR-0012). Home skills from
//! `~/.thirdshift/skills` are linked the same way.

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
    Home(PathBuf),
}

/// Resolve a skill name. An exact Factory directory name wins, then a
/// repository skill in `.agents/skills/<name>/SKILL.md` or
/// `.claude/skills/<name>/SKILL.md` that resolves inside the worktree, then
/// `~/.thirdshift/skills/<name>/SKILL.md` when that file resolves inside the
/// home skills directory. The Factory shorthand `thirdshift-<name>` is last.
/// A symlink cannot select instructions outside the directory being searched.
pub fn find(worktree: &Path, name: &str) -> Result<Skill> {
    let mut available = BTreeMap::new();
    for dir in SKILLS.dirs() {
        available.insert(dir.path().to_string_lossy().into_owned(), Source::Factory);
    }
    for (skill_name, path) in repository_skills(worktree)? {
        available
            .entry(skill_name)
            .or_insert(Source::Repository(path));
    }
    for (skill_name, path) in home_skills()? {
        available.entry(skill_name).or_insert(Source::Home(path));
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
                Source::Repository(path) | Source::Home(path) => path.clone(),
            },
            name: name.clone(),
        });
    }
    bail!(
        "unknown skill {name:?}; available skills: {}",
        available.keys().cloned().collect::<Vec<_>>().join(", ")
    )
}

/// `candidate` resolves to a file or directory inside `root`. Symlinks are
/// followed, so a skill that points outside the directory being searched is
/// not a match.
fn resolved_inside(root: &Path, candidate: &Path) -> bool {
    fs::canonicalize(candidate).is_ok_and(|target| target.starts_with(root))
}

/// Repository skills whose `SKILL.md` resolves inside `worktree`. `.agents`
/// is listed before `.claude`, and the caller keeps the first name.
fn repository_skills(worktree: &Path) -> Result<Vec<(String, PathBuf)>> {
    let root = fs::canonicalize(worktree)
        .with_context(|| format!("could not resolve {}", worktree.display()))?;
    let mut skills = Vec::new();
    for folder in [".agents/skills", ".claude/skills"] {
        let folder_path = worktree.join(folder);
        match fs::symlink_metadata(&folder_path) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error).with_context(|| format!("could not list {folder}")),
        }
        if !resolved_inside(&root, &folder_path) {
            continue;
        }
        let entries =
            fs::read_dir(&folder_path).with_context(|| format!("could not list {folder}"))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path().join("SKILL.md");
            if !resolved_inside(&root, &path) || !path.is_file() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|_| {
                anyhow::anyhow!(
                    "skill directory name is not UTF-8: {}",
                    entry.path().display()
                )
            })?;
            skills.push((name, path));
        }
    }
    Ok(skills)
}

/// Home skills, each `~/.thirdshift/skills/<name>/SKILL.md` that resolves
/// inside that directory. No home directory, or `HOME` unset, is none.
fn home_skills() -> Result<Vec<(String, PathBuf)>> {
    let Ok(home) = crate::config::home() else {
        return Ok(Vec::new());
    };
    let skills_dir = home.join(".thirdshift/skills");
    let entries = match fs::read_dir(&skills_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not list {}", skills_dir.display()));
        }
    };
    let root = match fs::canonicalize(&skills_dir) {
        Ok(root) => root,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("could not resolve {}", skills_dir.display()));
        }
    };
    let mut skills = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("could not list {}", skills_dir.display()))?;
        // One odd directory name must not make every skill lookup fail.
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !usable_home_skill_name(&name) {
            continue;
        }
        let path = entry.path().join("SKILL.md");
        if !resolved_inside(&root, &path) || !path.is_file() {
            continue;
        }
        skills.push((name, path));
    }
    Ok(skills)
}

/// A single path component that can be one line of `.git/info/exclude`.
fn usable_home_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
}

/// A gitignore line that matches only `path`. `path` starts with `/`.
fn exclude_literal(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    let last_non_space = chars.iter().rposition(|ch| *ch != ' ').unwrap_or(0);
    let mut escaped = String::new();
    for (index, ch) in chars.into_iter().enumerate() {
        if matches!(ch, '*' | '?' | '[' | ']' | '\\') || (ch == ' ' && index > last_non_space) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// The temp directory the Command wrote the Factory skills out to, once it
/// has.
static WRITTEN: Mutex<Option<TempDir>> = Mutex::new(None);

/// Link every Factory skill into `worktree`'s project skills for `adapter`,
/// `.claude/skills/` or `.agents/skills/`, writing them out first if the
/// Command has not yet, and make sure the repository's `.git/info/exclude`
/// keeps them out of git. Home skills are linked under their own names the
/// same way, except where the repository already has that skill. The links
/// are left in place: they go with the worktree.
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
    link_home_skills(worktree, &project_skills, dir)?;
    Ok(())
}

/// Link each home skill under its own name, unless the repository already has
/// that skill or the destination is some other entry. Each linked name is
/// excluded on its own, because home skills do not share the `thirdshift-`
/// prefix. The entry stays, as the Factory pattern does.
fn link_home_skills(worktree: &Path, project_skills: &CheckedDirectory, dir: &str) -> Result<()> {
    let skills = home_skills()?;
    if skills.is_empty() {
        return Ok(());
    }
    // Probe each name. Listing the project skills directory would fail when
    // it is searchable but not readable, which Factory linking allows.
    let root = fs::canonicalize(worktree)
        .with_context(|| format!("could not resolve {}", worktree.display()))?;
    for (name, skill_md) in skills {
        if repository_has_skill(&root, worktree, &name) {
            continue;
        }
        let Some(directory) = skill_md.parent() else {
            continue;
        };
        let destination = worktree.join(dir).join(&name);
        if !home_link_allowed(&destination, directory)? {
            continue;
        }
        exclude(
            &Git::new(worktree),
            &exclude_literal(&format!("/{dir}/{name}")),
        )?;
        project_skills.link(directory, Path::new(&name))?;
    }
    Ok(())
}

/// `worktree` has a committed skill named `name` in either project skills
/// directory. Missing paths and directories that cannot be searched are not.
fn repository_has_skill(root: &Path, worktree: &Path, name: &str) -> bool {
    [".agents/skills", ".claude/skills"]
        .into_iter()
        .any(|folder| {
            let path = worktree.join(folder).join(name).join("SKILL.md");
            resolved_inside(root, &path) && path.is_file()
        })
}

/// The destination is absent, or it is already this home skill's link and can
/// be refreshed. A repository file or directory, or a link to somewhere else,
/// stays.
fn home_link_allowed(destination: &Path, directory: &Path) -> Result<bool> {
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(true),
        Err(error) => {
            Err(error).with_context(|| format!("could not inspect {}", destination.display()))
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Ok(fs::read_link(destination).ok().as_deref() == Some(directory))
        }
        Ok(_) => Ok(false),
    }
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
