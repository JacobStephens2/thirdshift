//! Versioned, deliberately strict manifests. Grader data never enters a Trial.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum State {
    Proposed,
    Approved,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Session {
    CodeReview,
    SpecReview,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Task {
    pub format: u32,
    pub state: State,
    pub session: Session,
    pub repository: String,
    pub base: String,
    pub change: String,
    pub criteria: String,
    #[serde(default)]
    pub checks: Vec<Vec<String>>,
    pub inputs: BTreeMap<String, String>,
}

impl Task {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("task.toml");
        let task: Self = read(&path)?;
        version(task.format)?;
        commit_id(&task.base)?;
        commit_id(&task.change)?;
        if task.repository.trim().is_empty() || task.criteria.trim().is_empty() {
            bail!(
                "{}: repository and criteria must not be empty",
                path.display()
            );
        }
        for check in &task.checks {
            if check
                .first()
                .is_none_or(|command| command.trim().is_empty())
            {
                bail!(
                    "{}: checks must be nonempty argument vectors",
                    path.display()
                );
            }
        }
        for (destination, source) in &task.inputs {
            relative_path(destination)?;
            relative_path(source)?;
        }
        Ok(task)
    }
}

pub(super) fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("can't read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid manifest {}", path.display()))
}

pub(super) fn version(version: u32) -> Result<()> {
    if version != 1 {
        bail!("unsupported Eval format {version}; expected 1");
    }
    Ok(())
}

pub(super) fn name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        bail!("invalid Eval suite or task name: {name}");
    }
    Ok(())
}

pub(super) fn commit_id(id: &str) -> Result<()> {
    if !matches!(id.len(), 40 | 64)
        || !id
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        bail!("expected a full lowercase commit object ID: {id}");
    }
    Ok(())
}

pub(super) fn relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty() || matches!(part, "." | "..") || part.eq_ignore_ascii_case(".git")
        })
        || path.contains('\\')
    {
        bail!("expected a confined relative Eval path: {path}");
    }
    Ok(())
}
