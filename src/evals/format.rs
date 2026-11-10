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
    Process,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Provenance {
    Real,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Environment {
    Host,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Task {
    pub format: u32,
    pub state: State,
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<Environment>,
    pub repository: String,
    pub base: String,
    pub change: String,
    pub criteria: String,
    #[serde(default)]
    pub checks: Vec<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_grading: Option<ProcessGrading>,
    pub inputs: BTreeMap<String, String>,
    pub trial: Option<crate::trial::Inputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub difficulty: Option<Difficulty>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProcessGrading {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub real_fix: Option<RealFix>,
    pub test_files: Vec<String>,
    #[serde(default)]
    pub test_fakes: Vec<String>,
    pub fail_to_pass: Vec<Vec<String>>,
    pub pass_to_pass: Vec<Vec<String>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RealFix {
    pub pull_request: String,
    pub issue: String,
    pub branch: String,
    pub title: String,
    #[serde(default)]
    pub issue_title: String,
}

/// One replaceable A/A reading, with the setup that produced it.
#[derive(PartialEq, Eq, Deserialize, Serialize)]
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
        if task.session == Session::Process {
            anyhow::ensure!(
                task.provenance.is_some() && task.environment.is_some(),
                "process tasks require provenance and environment"
            );
            anyhow::ensure!(
                task.checks.is_empty(),
                "process tasks use process_grading, not checks"
            );
            let grading = task
                .process_grading
                .as_ref()
                .context("process tasks require process_grading")?;
            anyhow::ensure!(
                !grading.test_files.is_empty(),
                "process_grading requires test_files"
            );
            for file in grading.test_files.iter().chain(&grading.test_fakes) {
                relative_path(file)?;
            }
            for selectors in [&grading.fail_to_pass, &grading.pass_to_pass] {
                anyhow::ensure!(
                    !selectors.is_empty(),
                    "process_grading requires fail_to_pass and pass_to_pass selectors"
                );
                for selector in selectors {
                    anyhow::ensure!(
                        selector
                            .first()
                            .is_some_and(|command| !command.trim().is_empty()),
                        "process_grading selectors must be nonempty argument vectors"
                    );
                }
            }
        } else {
            anyhow::ensure!(
                task.process_grading.is_none(),
                "only process tasks use process_grading"
            );
        }
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
