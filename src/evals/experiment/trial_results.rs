//! Trial identity, retained result decoding, captures and Model Score updates.

pub(super) mod files;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use self::files::{Files, Native, directory, metadata};
use super::{grading, rechecks, store};
use crate::trial::{Outcome, Output};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Arm {
    Baseline,
    Candidate,
}

impl Arm {
    pub fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }

    pub fn from_trial_key(key: &str) -> Option<Self> {
        let name = key.split("--").nth(1)?;
        [Self::Baseline, Self::Candidate]
            .into_iter()
            .find(|arm| arm.name() == name)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TrialResult {
    pub suite: String,
    pub task: String,
    pub arm: Arm,
    pub trial: usize,
    pub outcome: Outcome,
    #[serde(default)]
    pub score: Option<grading::Score>,
    pub attempts: Vec<Output>,
}

impl TrialResult {
    pub fn key(&self) -> String {
        trial_key(&self.suite, &self.task, self.arm, self.trial)
    }
}

pub(crate) fn trial_key(suite: &str, name: &str, arm: Arm, index: usize) -> String {
    // Names cannot contain NUL. Hash the delimited tuple so names with
    // `--` stay distinct and long names cannot exceed a filename limit.
    let task = Sha256::new()
        .chain_update(suite.as_bytes())
        .chain_update([0])
        .chain_update(name.as_bytes())
        .finalize();
    format!("{task:x}--{}--{}", arm.name(), index)
}

pub(super) struct Terminal {
    path: PathBuf,
    result: TrialResult,
}

/// A saved assessment key, resolved only against selected terminal entries.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct ScoreTarget(String);

impl Terminal {
    pub fn result(&self) -> &TrialResult {
        &self.result
    }

    pub fn target(&self) -> Result<ScoreTarget> {
        Ok(ScoreTarget(
            self.path
                .file_name()
                .context("missing Trial filename")?
                .to_str()
                .context("invalid Trial filename")?
                .to_owned(),
        ))
    }
}

pub(super) fn terminal(root: &Path) -> Result<Vec<Terminal>> {
    read_terminal(&Native, root)
}

pub(super) fn apply_scores(
    entries: &mut [Terminal],
    scores: &BTreeMap<ScoreTarget, super::model_grader::Score>,
) -> Result<()> {
    ensure!(
        scores.len() == entries.len(),
        "saved Grader Scores do not cover Trials"
    );
    for entry in entries {
        let score = scores
            .get(&entry.target()?)
            .context("saved Grader Score missing")?;
        entry
            .result
            .score
            .as_mut()
            .context("Trial has no mechanical Score")?
            .with_model(score.clone());
        store::write(&entry.path, &entry.result)?;
    }
    Ok(())
}

pub(super) fn read_terminal(files: &impl Files, root: &Path) -> Result<Vec<Terminal>> {
    let mut results = Vec::new();
    for entry in directory(files, &root.join("results"))? {
        if entry.kind.is_file() && entry.path.extension().is_some_and(|ext| ext == "json") {
            let result = serde_json::from_slice(&files.read(&entry.path)?)?;
            results.push(Terminal {
                path: entry.path,
                result,
            });
        }
    }
    Ok(results)
}

pub(super) fn effective(entries: Vec<Terminal>, history: &rechecks::History) -> Vec<TrialResult> {
    entries
        .into_iter()
        .map(|entry| {
            let mut result = entry.result;
            let key = result.key();
            if let Some(score) = &mut result.score {
                score.rechecks = history.get(&key).cloned().unwrap_or_default();
            }
            result
        })
        .collect()
}

pub(super) struct CapturedTrial {
    pub path: PathBuf,
    pub id: String,
    pub pending: Option<TrialResult>,
    finished: bool,
}

impl CapturedTrial {
    pub fn active_eligible(&self) -> bool {
        !self.finished
    }
}

pub(super) struct Capture {
    terminal: Vec<Terminal>,
    terminal_keys: BTreeSet<String>,
    trials: Vec<CapturedTrial>,
}

pub(super) struct Progress {
    pub results: Vec<TrialResult>,
    pub unfinished_trials: Vec<TrialResult>,
    pub minutes: store::Minutes,
    pub voids: store::Voids,
}

pub(super) fn capture(files: &impl Files, root: &Path) -> Result<Capture> {
    // Membership precedes pending sampling and is never probed again.
    let terminal = read_terminal(files, root)?;
    let terminal_keys: BTreeSet<_> = terminal.iter().map(|entry| entry.result.key()).collect();
    let mut trials = Vec::new();
    for entry in directory(files, &root.join("trials"))? {
        let id = entry
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let pending = metadata(files, &entry.path.join("pending.json"))?;
        let finished = terminal_keys.contains(&id);
        trials.push(CapturedTrial {
            path: entry.path,
            id,
            pending,
            finished,
        });
    }
    Ok(Capture {
        terminal,
        terminal_keys,
        trials,
    })
}

impl Capture {
    pub fn trials(&self) -> &[CapturedTrial] {
        &self.trials
    }

    pub fn into_progress(self, history: &rechecks::History) -> Progress {
        let mut minutes = store::Minutes::default();
        let mut voids = store::Voids::default();
        let mut unfinished_trials = Vec::new();
        for trial in self.trials {
            if let Some(result) = trial.pending {
                // Pending history is the sole accounting source, even for
                // finished Trials whose terminal Score was later updated.
                minutes.include(&result);
                voids.include(&result);
                if !self.terminal_keys.contains(&result.key()) {
                    unfinished_trials.push(result);
                }
            }
        }
        Progress {
            results: effective(self.terminal, history),
            unfinished_trials,
            minutes,
            voids,
        }
    }
}

pub(super) struct Identity<'a> {
    pub suite: &'a str,
    pub task: &'a str,
    pub arm: Arm,
    pub trial: usize,
}

impl Identity<'_> {
    pub fn key(&self) -> String {
        trial_key(self.suite, self.task, self.arm, self.trial)
    }
}

pub(super) fn finished(directory: &Path, identity: &Identity<'_>) -> bool {
    directory
        .join("results")
        .join(format!("{}.json", identity.key()))
        .exists()
}

#[cfg(test)]
mod tests;
