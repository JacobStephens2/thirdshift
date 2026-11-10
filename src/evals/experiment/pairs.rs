//! Base-tests and cross-tests for each pair of Trials: Baseline Trial *k*
//! with Candidate Trial *k*. Each Arm's own new tests run on the unchanged
//! base and on the other Arm's final commit. They are evidence, never Scores.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::evidence::Started;
use super::grading::process::{self, Run, Source};
use super::trial_results::files::{Files, directory};
use super::{Arm, Task, TrialResult, store, trial_results};
use crate::evals::{format, repository};
use crate::sandbox::code_grader::Check;
use crate::{interrupt, progress};

#[derive(Serialize, Deserialize)]
pub(crate) struct Pair {
    pub suite: String,
    pub task: String,
    pub trial: usize,
    pub baseline: Side,
    pub candidate: Side,
}

/// One Arm's new tests, run on the base and against the other Arm.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Side {
    /// Files the Arm's final commit adds or changes that match `new_tests`.
    pub test_files: Vec<String>,
    /// Null when the Arm has no new tests or its evidence could not run.
    pub fails_on_base: Option<bool>,
    pub passes_on_other: Option<bool>,
    /// A cross-test failure: something to look into, not a defect.
    pub disagreement: bool,
    pub base: Vec<Check>,
    pub cross: Vec<Check>,
    pub error: Option<String>,
}

/// Run evidence for every finished pair still without it. Retained pairs are
/// never run again, so a resumed Experiment continues where it stopped.
pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let bar = &experiment.description.bar;
    let timeout = Duration::from_secs(bar.timeout_seconds);
    let pairs = experiment.directory.join("pairs");
    for task in &experiment.tasks {
        let Some(new_tests) = task
            .manifest
            .process_grading
            .as_ref()
            .and_then(|grading| grading.new_tests.as_ref())
        else {
            continue;
        };
        for index in 0..bar.trials_per_task[&task.suite] {
            let path = pairs.join(format!(
                "{}.json",
                trial_results::pair_key(&task.suite, &task.name, index)
            ));
            let finished = [Arm::Baseline, Arm::Candidate].map(|arm| {
                trial_results::finished(
                    &experiment.directory,
                    &trial_results::Identity {
                        suite: &task.suite,
                        task: &task.name,
                        arm,
                        trial: index,
                    },
                )
            });
            if path.exists() || finished.contains(&false) {
                continue;
            }
            interrupt::check()?;
            progress::step(format!(
                "Pair evidence {}/{} #{index}: base-tests and cross-tests",
                task.suite, task.name
            ));
            let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
            let [baseline, candidate] = [Arm::Baseline, Arm::Candidate]
                .map(|arm| last_commit(&experiment.directory, task, arm, index));
            let evidence = Evidence {
                task,
                new_tests,
                timeout,
                caps: &caps,
            };
            let pair = Pair {
                suite: task.suite.clone(),
                task: task.name.clone(),
                trial: index,
                baseline: evidence.side(&baseline, &candidate)?,
                candidate: evidence.side(&candidate, &baseline)?,
            };
            fs::create_dir_all(&pairs)?;
            store::write(&path, &pair)?;
        }
    }
    Ok(())
}

/// Retained pairs in a stable order, for the Experiment view.
pub(super) fn read(files: &impl Files, root: &Path) -> Result<Vec<Pair>> {
    let mut pairs: Vec<Pair> = Vec::new();
    for entry in directory(files, &root.join("pairs"))? {
        if entry.kind.is_file() && entry.path.extension().is_some_and(|ext| ext == "json") {
            pairs.push(serde_json::from_slice(&files.read(&entry.path)?)?);
        }
    }
    pairs.sort_by(|a, b| (&a.suite, &a.task, a.trial).cmp(&(&b.suite, &b.task, b.trial)));
    Ok(pairs)
}

/// One-line summaries for the Experiment's text view, numbering pairs from 1.
pub(super) fn lines(pairs: &[Pair]) -> String {
    let mut text = String::new();
    for pair in pairs {
        for (arm, other, side) in [
            ("Baseline", "Candidate", &pair.baseline),
            ("Candidate", "Baseline", &pair.candidate),
        ] {
            let mut findings = Vec::new();
            if let Some(error) = &side.error {
                findings.push(format!("evidence unavailable: {error}"));
            }
            if side.fails_on_base == Some(false) {
                findings.push("pass on the base".to_owned());
            }
            if side.disagreement {
                findings.push(format!("fail on the {other}'s commit (disagreement)"));
            }
            for finding in findings {
                text.push_str(&format!(
                    "\nPair {}/{} #{} · {arm} tests {finding}",
                    pair.suite,
                    pair.task,
                    pair.trial + 1
                ));
            }
        }
    }
    text
}

/// A finished Trial's final commit, in its last attempt's repository.
struct Commit {
    repository: PathBuf,
    commit: String,
}

fn last_commit(experiment: &Path, task: &Task, arm: Arm, index: usize) -> Result<Commit> {
    let key = trial_results::trial_key(&task.suite, &task.name, arm, index);
    let result: TrialResult = serde_json::from_slice(&fs::read(
        experiment.join("results").join(format!("{key}.json")),
    )?)?;
    let last = result
        .attempts
        .len()
        .checked_sub(1)
        .context("Trial has no attempts")?;
    for entry in fs::read_dir(experiment.join("trials").join(&key))? {
        let attempt = entry?.path();
        let started = attempt.join("started.json");
        if !attempt
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("attempt-"))
            || !started.is_file()
        {
            continue;
        }
        let started: Started = serde_json::from_slice(&fs::read(started)?)?;
        if started.index == last {
            let repository = attempt.join("repository");
            let commit = repository::bytes_until(
                &repository,
                &["rev-parse", "--verify", "HEAD^{commit}"],
                Instant::now() + Duration::from_secs(60),
            )
            .context("Trial repository is no longer retained")?;
            return Ok(Commit {
                repository,
                commit: std::str::from_utf8(&commit)?.trim().to_owned(),
            });
        }
    }
    anyhow::bail!("Trial's last attempt is no longer retained")
}

struct Evidence<'a> {
    task: &'a Task,
    new_tests: &'a format::NewTests,
    timeout: Duration,
    caps: &'a crate::scope::Caps,
}

impl Evidence<'_> {
    /// `own`'s new tests on the base, then on `other`'s final commit. Each run
    /// has the Bar's timeout, shared with preparing its snapshot.
    fn side(&self, own: &Result<Commit>, other: &Result<Commit>) -> Result<Side> {
        let mut side = Side::default();
        let result = (|| -> Result<()> {
            let own = own.as_ref().map_err(|error| anyhow::anyhow!("{error:#}"))?;
            let manifest = &self.task.manifest;
            let deadline = Instant::now() + self.timeout;
            side.test_files =
                process::changed(&own.repository, &manifest.base, &own.commit, deadline)?
                    .into_iter()
                    .filter(|path| self.new_tests.matches(path))
                    .collect();
            if side.test_files.is_empty() {
                return Ok(());
            }
            let commands: Vec<_> = side
                .test_files
                .iter()
                .map(|path| self.new_tests.command_for(path))
                .collect();
            let tests = || Source {
                repository: &own.repository,
                commit: &own.commit,
            };
            let source = self.task.directory.join(&manifest.repository);
            side.base = process::overlaid(
                Source {
                    repository: &source,
                    commit: &manifest.base,
                },
                tests(),
                &side.test_files,
                &commands,
                Run {
                    name: "base-test",
                    deadline,
                    caps: self.caps,
                },
            )?;
            side.fails_on_base = Some(!side.base.iter().all(|check| check.passed));
            let other = other
                .as_ref()
                .map_err(|error| anyhow::anyhow!("other Arm: {error:#}"))?;
            side.cross = process::overlaid(
                Source {
                    repository: &other.repository,
                    commit: &other.commit,
                },
                tests(),
                &side.test_files,
                &commands,
                Run {
                    name: "cross-test",
                    deadline: Instant::now() + self.timeout,
                    caps: self.caps,
                },
            )?;
            let passes = side.cross.iter().all(|check| check.passed);
            side.passes_on_other = Some(passes);
            side.disagreement = !passes;
            Ok(())
        })();
        if let Err(error) = result {
            // Interruption stops the Experiment; other failures are evidence.
            interrupt::check()?;
            side.error = Some(format!("{error:#}"));
        }
        Ok(side)
    }
}
