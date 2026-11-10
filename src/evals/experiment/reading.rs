//! One captured observation supplies every retained Experiment projection.

use std::io::BufReader;

use super::evidence::{RunningTrial, Started};
use super::session_evidence::{Inventory, SessionLog};
use super::store::{Minutes, State, Stored, TaskName, View, Voids};
use super::trial_results::files::{Files, Native, metadata};
use super::{Arm, TrialResult, trial_results, trial_sessions, usage};
use anyhow::Result;

#[derive(Default)]
struct Observation {
    results: Vec<TrialResult>,
    running_trials: Vec<RunningTrial>,
    sessions: Vec<SessionLog>,
    unfinished_trials: Vec<TrialResult>,
    minutes: Minutes,
    voids: Voids,
    usage: usage::Arms,
    pairs: Vec<super::pairs::Pair>,
    panel: Vec<super::panel::PanelPair>,
    people: super::pairwise::People,
}

pub(super) fn read(stored: &Stored) -> Result<View> {
    Ok(Observation::capture(&Native, stored)?.project(stored))
}

impl Observation {
    fn capture(files: &impl Files, stored: &Stored) -> Result<Self> {
        let capture = trial_results::capture(files, &stored.directory)?;
        let mut observation = Self::default();
        let inventory = Inventory::new(files, &stored.directory)?;
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
            for attempt in trial_sessions::trial(files, &trial.path)? {
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
                // Usage is independent of readable evidence: a spans-only
                // Session still contributes, as do retries and Resumes.
                if let Some(arm) = Arm::from_trial_key(id) {
                    for session in &attempt.sessions {
                        if let Some(path) = &session.spans {
                            observation
                                .usage
                                .include(arm, BufReader::new(files.open(path)?))?;
                        }
                    }
                }
                observation
                    .sessions
                    .extend(inventory.listing(files, &attempt, id, index)?);
            }
        }
        let progress = capture.into_progress(&stored.rechecks);
        observation.results = progress.results;
        observation.unfinished_trials = progress.unfinished_trials;
        super::leaks::apply(&mut observation.results, &stored.leak_decisions);
        super::leaks::apply(&mut observation.unfinished_trials, &stored.leak_decisions);
        observation.minutes = progress.minutes;
        observation.voids = progress.voids;
        if !stored.leak_decisions.is_empty() {
            observation.voids = Voids::default();
            for result in observation
                .results
                .iter()
                .chain(&observation.unfinished_trials)
            {
                observation.voids.include(result);
            }
        }
        observation.pairs = super::pairs::read(files, &stored.directory)?;
        observation.panel =
            super::panel::read(files, &stored.directory, &stored.description, &stored.tasks)?;
        super::panel::attach(&mut observation.pairs, &observation.panel);
        observation.people = super::pairwise::people(
            files,
            &stored.directory,
            &observation.pairs,
            stored.description.bar.seed,
        )?;
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
            format: 10,
            name: stored.name(),
            name_set: stored.name.is_some(),
            description: stored.description.clone(),
            state: stored.state,
            pricing: stored.pricing.clone(),
            accounting: super::accounting::Report::of(&self.results, &self.unfinished_trials),
            details: {
                let mut details = stored.details.clone();
                details.resume_command = details.resume_command.as_deref().and_then(|command| {
                    super::options::resume_command(command, stored.description.bar.levels)
                });
                details
            },
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
            outcomes: super::store::Outcomes::of(&self.results, &stored.tasks),
            leak_hits: self
                .results
                .iter()
                .chain(&self.unfinished_trials)
                .flat_map(|result| result.leaks.clone())
                .collect(),
            results: self.results,
            unfinished_trials: self.unfinished_trials,
            verdict: stored.verdict.clone(),
            pending_rechecks: stored.pending_rechecks.clone(),
            minutes: self.minutes,
            usage: self.usage,
            findings: self.voids.findings(),
            voids: self.voids,
            pairs: self.pairs,
            panel: self.panel,
            people: self.people,
        }
    }
}

#[cfg(test)]
mod tests;
