//! Suite declarations own selection; the runner need not know session names.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::format::{self, Session, State, Task};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Dependencies {
    factory_skills: Vec<String>,
    session_prompts: Vec<String>,
    harness_defaults: Vec<String>,
    model_defaults: Vec<String>,
    effort_defaults: Vec<String>,
}

impl Dependencies {
    fn paths(&self) -> impl Iterator<Item = &str> {
        [
            &self.factory_skills,
            &self.session_prompts,
            &self.harness_defaults,
            &self.model_defaults,
            &self.effort_defaults,
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    format: u32,
    pub(super) trials_per_arm: NonZeroUsize,
    dependencies: Dependencies,
}

impl Manifest {
    pub fn load(directory: &Path) -> Result<Self> {
        let manifest: Self = format::read(&directory.join("suite.toml"))?;
        format::version(manifest.format)?;
        for path in manifest.dependencies.paths() {
            if path.contains(['*', '?', '[', ']']) {
                bail!(
                    "Eval dependencies use exact paths or directory prefixes, not wildcards: {path}"
                );
            }
            format::relative_path(path.strip_suffix('/').unwrap_or(path))?;
        }
        Ok(manifest)
    }
}

#[derive(Serialize)]
pub(super) struct Suite {
    pub(super) name: String,
    trials_per_arm: NonZeroUsize,
    dependencies: Dependencies,
    pub(super) tasks: Vec<Entry>,
}

#[derive(Serialize)]
pub(super) struct Entry {
    pub(super) name: String,
    pub(super) state: State,
    session: Session,
}

pub(super) fn list(root: &Path) -> Result<Vec<Suite>> {
    directories(root)?
        .into_iter()
        .map(|(name, directory)| {
            let manifest = Manifest::load(&directory)?;
            let tasks = directories(&directory.join("tasks"))?
                .into_iter()
                .map(|(name, directory)| {
                    let task = Task::load(&directory)?;
                    Ok(Entry {
                        name,
                        state: task.state,
                        session: task.session,
                    })
                })
                .collect::<Result<_>>()?;
            Ok(Suite {
                name,
                trials_per_arm: manifest.trials_per_arm,
                dependencies: manifest.dependencies,
                tasks,
            })
        })
        .collect()
}

pub(super) fn touched(root: &Path, paths: &[String]) -> Result<Vec<String>> {
    let mut selected = Vec::new();
    for (name, directory) in directories(root)? {
        let manifest = Manifest::load(&directory)?;
        if manifest.dependencies.paths().any(|dependency| {
            paths.iter().any(|path| {
                if dependency.ends_with('/') {
                    path.starts_with(dependency)
                } else {
                    path == dependency
                }
            })
        }) {
            selected.push(name);
        }
    }
    Ok(selected)
}

pub(super) fn directories(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("can't read Eval folder {}", root.display()));
        }
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Eval folder name is not UTF-8"))?;
            format::name(&name)?;
            directories.push((name, entry.path()));
        }
    }
    directories.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(directories)
}
