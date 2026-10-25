//! One captured observation supplies every retained Experiment projection.

use std::collections::BTreeSet;
use std::io::BufReader;

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::evidence::{RunningTrial, SessionLog, Started};
use super::store::{Minutes, State, Stored, TaskName, View, Voids};
use super::trial_results::files::{Files, Native, directory, metadata, optional};
use super::{Arm, TrialResult, trial_results, usage};

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
        let capture = trial_results::capture(files, &stored.directory)?;
        let mut observation = Self::default();
        let root = files.canonicalize(&stored.directory)?;
        for trial in capture.trials() {
            let id = &trial.id;
            let pending = &trial.pending;
            if stored.state == State::Running
                && trial.active_eligible()
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
                if let Some(arm) = Arm::from_trial_key(id) {
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
        }
        let progress = capture.into_progress(&stored.rechecks);
        observation.results = progress.results;
        observation.unfinished_trials = progress.unfinished_trials;
        observation.minutes = progress.minutes;
        observation.voids = progress.voids;
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
