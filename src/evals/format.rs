//! Versioned, deliberately strict manifests. Grader data never enters a Trial.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum State {
    Proposed,
    Approved,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
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
    pub trial: Option<crate::trial::Inputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
}

/// One replaceable A/A reading, with the setup that produced it.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Difficulty {
    pub passes: usize,
    pub trials: usize,
    pub experiment: String,
    pub commit: String,
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl Difficulty {
    pub fn measured(&self) -> bool {
        self.trials >= 6
    }

    pub fn discriminating(&self) -> bool {
        self.measured() && (0.15..=0.95).contains(&(self.passes as f64 / self.trials as f64))
    }
}

impl Task {
    pub fn load(directory: &Path) -> Result<Self> {
        let path = directory.join("task.toml");
        let task: Self = read(&path)?;
        version(task.format)?;
        commit_id(&task.base)?;
        commit_id(&task.change)?;
        if let Some(difficulty) = &task.difficulty {
            anyhow::ensure!(
                difficulty.passes <= difficulty.trials,
                "Difficulty passes exceed Trials"
            );
            commit_id(&difficulty.commit)?;
        }
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

/// Publish one edited manifest atomically, preserving its TOML formatting.
/// The caller holds the owning suite's lock across reading and writing.
pub(super) fn write(path: &Path, document: &toml_edit::DocumentMut) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("missing Eval task folder")?)?;
    file.write_all(document.to_string().as_bytes())?;
    file.flush()?;
    file.persist(path)?;
    Ok(())
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
