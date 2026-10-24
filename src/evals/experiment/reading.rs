//! One captured observation supplies every retained Experiment projection.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use super::evidence::{RunningTrial, SessionLog, Started};
use super::store::{Minutes, State, Stored, TaskName, View, Voids};
use super::{Arm, TrialResult, usage};

// Private I/O seam: production callers only request the existing View. Tests
// can count sampling and publish files between a sample and its projection.
trait Files {
    fn entries(&self, path: &Path) -> io::Result<Vec<Entry>>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn open(&self, path: &Path) -> io::Result<File>;
}

struct Entry {
    path: PathBuf,
    kind: fs::FileType,
}

struct Native;

impl Files for Native {
    fn entries(&self, path: &Path) -> io::Result<Vec<Entry>> {
        fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok(Entry {
                    path: entry.path(),
                    kind: entry.file_type()?,
                })
            })
            .collect()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }

    fn open(&self, path: &Path) -> io::Result<File> {
        File::open(path)
    }
}

fn optional<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn metadata<T: DeserializeOwned>(files: &impl Files, path: &Path) -> Result<Option<T>> {
    optional(files.read(path))?
        .map(|bytes| {
            serde_json::from_slice(&bytes).with_context(|| format!("can't read {}", path.display()))
        })
        .transpose()
}

fn directory(files: &impl Files, path: &Path) -> Result<Vec<Entry>> {
    let mut entries = optional(files.entries(path))?.unwrap_or_default();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[derive(Default)]
struct Observation {
    results: Vec<TrialResult>,
    running_trials: Vec<RunningTrial>,
    sessions: Vec<SessionLog>,
    unfinished_trials: Vec<TrialResult>,
    minutes: Minutes,
    voids: Voids,
    usage: usage::Arms,
}

pub(super) fn read(stored: &Stored) -> Result<View> {
    Ok(Observation::capture(&Native, stored)?.project(stored))
}

impl Observation {
    fn capture(files: &impl Files, stored: &Stored) -> Result<Self> {
        let mut observation = Self::default();
        // Capture terminal membership first. Nothing below probes terminal
        // paths again, even if a worker finishes while we sample a Trial.
        for entry in directory(files, &stored.directory.join("results"))? {
            if !entry.kind.is_file() || !entry.path.extension().is_some_and(|ext| ext == "json") {
                continue;
            }
            let mut result: TrialResult = serde_json::from_slice(&files.read(&entry.path)?)?;
            let key = result.key();
            if let Some(score) = &mut result.score {
                score.rechecks = stored.rechecks.get(&key).cloned().unwrap_or_default();
            }
            observation.results.push(result);
        }
        let terminal: BTreeSet<_> = observation.results.iter().map(TrialResult::key).collect();
        let root = files.canonicalize(&stored.directory)?;
        for trial in directory(files, &stored.directory.join("trials"))? {
            let id = trial
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let pending: Option<TrialResult> = metadata(files, &trial.path.join("pending.json"))?;
            if stored.state == State::Running
                && !terminal.contains(&id)
                && let Some(started) = metadata::<Started>(files, &trial.path.join("active.json"))?
            {
                observation.running_trials.push(RunningTrial {
                    id: id.clone(),
                    started_at: started.started_at,
                });
            }
            for attempt in directory(files, &trial.path)? {
                if !attempt.kind.is_dir()
                    || !attempt
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("attempt-")
                {
                    continue;
                }
                let index = match metadata::<Started>(files, &attempt.path.join("started.json"))? {
                    Some(started) => started.index,
                    None => pending
                        .as_ref()
                        .and_then(|result| {
                            result.attempts.iter().position(|output| {
                                output
                                    .log
                                    .as_ref()
                                    .is_some_and(|log| log.starts_with(&attempt.path))
                            })
                        })
                        .unwrap_or(0),
                };
                let folder = attempt.path.join("session");
                let entries = directory(files, &folder)?;
                // Usage is independent of readable evidence: a spans-only
                // Session still contributes, as do retries and Resumes.
                if let Some(arm) = Arm::from_trial_key(&id) {
                    for entry in &entries {
                        if entry
                            .path
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .ends_with(".spans.jsonl")
                        {
                            observation
                                .usage
                                .include(arm, BufReader::new(files.open(&entry.path)?))?;
                        }
                    }
                }
                let logs: BTreeSet<_> = entries
                    .iter()
                    .filter(|entry| entry.kind.is_file())
                    .map(|entry| &entry.path)
                    .filter(|path| {
                        let name = path.file_name().unwrap().to_string_lossy();
                        name.ends_with(".jsonl") && !name.ends_with(".spans.jsonl")
                    })
                    .collect();
                if logs.is_empty() {
                    continue;
                }
                let Some(confined) = optional(files.canonicalize(&folder))? else {
                    continue;
                };
                if !confined.starts_with(&root) {
                    continue;
                }
                for path in &logs {
                    let selected = crate::logs::session::path(path);
                    if selected != **path && logs.contains(&selected) {
                        continue;
                    }
                    let Some(canonical) = optional(files.canonicalize(path))? else {
                        continue;
                    };
                    if !canonical.starts_with(&confined) {
                        continue;
                    }
                    // Hash the original relative log path, preserving the ID
                    // when an old raw log becomes a Session trace.
                    let original = crate::logs::session::original(path);
                    let session_id = format!(
                        "{:x}",
                        Sha256::digest(
                            original
                                .strip_prefix(&stored.directory)?
                                .as_os_str()
                                .as_encoded_bytes()
                        )
                    );
                    observation.sessions.push(SessionLog {
                        id: session_id,
                        trial_id: id.clone(),
                        attempt: index,
                        trace: path
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .ends_with(".logs.jsonl"),
                        path: (*path).clone(),
                    });
                }
            }
            if let Some(result) = pending {
                observation.minutes.include(&result);
                observation.voids.include(&result);
                if !terminal.contains(&result.key()) {
                    observation.unfinished_trials.push(result);
                }
            }
        }
        observation.running_trials.sort_by(|a, b| a.id.cmp(&b.id));
        observation
            .sessions
            .sort_by(|a, b| (&a.trial_id, a.attempt, &a.id).cmp(&(&b.trial_id, b.attempt, &b.id)));
        Ok(observation)
    }

    // All filesystem work has ended. Projection can only use captured values
    // and the Experiment metadata already loaded by Stored.
    fn project(self, stored: &Stored) -> View {
        View {
            description: stored.description.clone(),
            state: stored.state,
            details: stored.details.clone(),
            created_at: stored.created_at.clone(),
            tasks: stored
                .tasks
                .iter()
                .map(|task| TaskName {
                    suite: task.suite.clone(),
                    name: task.name.clone(),
                })
                .collect(),
            running_trials: self.running_trials,
            sessions: self.sessions,
            results: self.results,
            unfinished_trials: self.unfinished_trials,
            verdict: stored.verdict.clone(),
            pending_rechecks: stored.pending_rechecks.clone(),
            minutes: self.minutes,
            usage: self.usage,
            findings: self.voids.findings(),
            voids: self.voids,
        }
    }
}

#[cfg(test)]
mod tests;
