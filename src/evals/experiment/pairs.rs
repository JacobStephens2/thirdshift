//! Base-tests and cross-tests for each pair of Trials: Baseline Trial *k*
//! with Candidate Trial *k*. Each Arm's own new tests run on the unchanged
//! base and on the other Arm's final commit. They are evidence, never Scores.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::evidence::Started;
use super::grading::process::{self, GraderRun, Source};
use super::trial_results::files::{Files, Native, directory, metadata};
use super::{Arm, Task, TrialResult, store, trial_results, trial_sessions};
use crate::evals::format;
use crate::sandbox::code_grader::{Check, Ran};
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
    let mut caps = None;
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
            let identities = [Arm::Baseline, Arm::Candidate].map(|arm| trial_results::Identity {
                suite: &task.suite,
                task: &task.name,
                arm,
                trial: index,
            });
            if path.exists()
                || identities
                    .iter()
                    .any(|identity| !trial_results::finished(&experiment.directory, identity))
            {
                continue;
            }
            interrupt::check()?;
            progress::step(format!(
                "Pair evidence {}/{} #{}: base-tests and cross-tests",
                task.suite,
                task.name,
                index + 1
            ));
            let caps = match &mut caps {
                Some(caps) => caps,
                None => caps.insert(crate::config::UserConfig::load()?.trial_caps.with_env()?),
            };
            let [baseline, candidate] = identities.map(|identity| {
                last_commit(&experiment.directory, &identity).map_err(|error| format!("{error:#}"))
            });
            let evidence = Evidence {
                task,
                new_tests,
                timeout,
                caps,
            };
            let pair = Pair {
                suite: task.suite.clone(),
                task: task.name.clone(),
                trial: index,
                baseline: evidence.side(resolved(&baseline), resolved(&candidate))?,
                candidate: evidence.side(resolved(&candidate), resolved(&baseline))?,
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
            match side.fails_on_base {
                Some(false) => findings.push("pass on the base".to_owned()),
                None if !side.base.is_empty() => {
                    findings.push("did not finish on the base".to_owned());
                }
                _ => {}
            }
            match side.passes_on_other {
                Some(false) => {
                    findings.push(format!("fail on the {other}'s commit (disagreement)"));
                }
                None if !side.cross.is_empty() => {
                    findings.push(format!("did not finish on the {other}'s commit"));
                }
                _ => {}
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

fn last_commit(experiment: &Path, identity: &trial_results::Identity<'_>) -> Result<Commit> {
    let key = identity.key();
    let result: TrialResult = serde_json::from_slice(&fs::read(
        experiment.join("results").join(format!("{key}.json")),
    )?)?;
    let last = result
        .attempts
        .len()
        .checked_sub(1)
        .context("Trial has no attempts")?;
    for attempt in trial_sessions::trial(&Native, &experiment.join("trials").join(&key))? {
        let started = metadata::<Started>(&Native, &attempt.path.join("started.json"))?;
        if started.is_some_and(|started| started.index == last) {
            let repository = attempt.path.join("repository");
            let commit = process::head(&repository, Instant::now() + Duration::from_secs(60))
                .context("Trial repository is no longer retained")?;
            return Ok(Commit { repository, commit });
        }
    }
    anyhow::bail!("Trial's last attempt is no longer retained")
}

fn resolved(commit: &Result<Commit, String>) -> Result<&Commit, String> {
    commit.as_ref().map_err(Clone::clone)
}

struct Evidence<'a> {
    task: &'a Task,
    new_tests: &'a format::NewTests,
    timeout: Duration,
    caps: &'a crate::scope::Caps,
}

/// What a run's checks say about the tests: `Some(false)` when one ran to a
/// failing exit, `Some(true)` when all ran and passed, and `None` when the
/// rest timed out or never started, which says nothing about the code.
fn outcome(ran: &[Ran]) -> Option<bool> {
    if ran.iter().any(|ran| ran.concluded && !ran.check.passed) {
        Some(false)
    } else {
        ran.iter().all(|ran| ran.concluded).then_some(true)
    }
}

impl Evidence<'_> {
    /// `own`'s new tests on the base, then on `other`'s final commit. Each run
    /// has the Bar's timeout, shared with preparing its snapshot.
    fn side(&self, own: Result<&Commit, String>, other: Result<&Commit, String>) -> Result<Side> {
        let mut side = Side::default();
        let result = (|| -> Result<()> {
            let own = own.map_err(anyhow::Error::msg)?;
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
            let base = process::overlaid(
                Source {
                    repository: &source,
                    commit: &manifest.base,
                },
                tests(),
                &side.test_files,
                &commands,
                GraderRun {
                    name: "base-test",
                    deadline,
                    caps: self.caps,
                },
            )?;
            side.fails_on_base = outcome(&base).map(|passed| !passed);
            side.base = base.into_iter().map(|ran| ran.check).collect();
            let other = other.map_err(|error| anyhow::anyhow!("other Arm: {error}"))?;
            let cross = process::overlaid(
                Source {
                    repository: &other.repository,
                    commit: &other.commit,
                },
                tests(),
                &side.test_files,
                &commands,
                GraderRun {
                    name: "cross-test",
                    deadline: Instant::now() + self.timeout,
                    caps: self.caps,
                },
            )?;
            side.passes_on_other = outcome(&cross);
            side.disagreement = side.passes_on_other == Some(false);
            side.cross = cross.into_iter().map(|ran| ran.check).collect();
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
