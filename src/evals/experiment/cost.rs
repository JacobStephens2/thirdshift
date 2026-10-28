//! What an Experiment's remaining Trials may cost, shown before the first of
//! them runs. A terminal confirms it; `--yes` and piped stdin go ahead.

use std::collections::BTreeMap;
use std::fs;
use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Result, bail};

use super::trial_results::files::Native;
use super::{Arm, TrialResult, store, trial_results, workers};
use crate::progress;
use crate::trial::Outcome;

const QUESTION: &str = "Run these Trials? [y/N] ";

/// Show the remaining Trials' cost, then ask on a terminal unless `yes`.
/// Declining returns an error before any Arm is built.
pub(super) fn confirm(experiment: &store::Stored, experiments: &Path, yes: bool) -> Result<()> {
    let Some(plan) = Plan::new(
        experiment,
        &history(experiments, &experiment.description.id),
    ) else {
        return Ok(());
    };
    for line in plan.lines() {
        progress::step(line);
    }
    if yes || !std::io::stdin().is_terminal() {
        return Ok(());
    }
    let mut stderr = std::io::stderr();
    let _ = write!(stderr, "{QUESTION}");
    let _ = stderr.flush();
    let answer = crate::terminal::line()?;
    if answer.is_none() {
        let _ = writeln!(stderr);
    }
    match answer.as_deref().map(str::trim) {
        Some("y" | "yes") => Ok(()),
        _ => bail!("Experiment declined; no Trials were run"),
    }
}

/// Mean minutes per completed Trial, by suite and Harness.
type History = BTreeMap<(String, String), Mean>;

#[derive(Clone, Copy, Default)]
struct Mean {
    minutes: f64,
    trials: usize,
}

impl Mean {
    fn include(&mut self, minutes: f64) {
        self.minutes += minutes;
        self.trials += 1;
    }

    fn get(self) -> f64 {
        self.minutes / self.trials as f64
    }
}

/// Completed, non-void Trials of this machine's other Experiments. An
/// unreadable Experiment only narrows the estimate, so it is left out.
fn history(experiments: &Path, current: &str) -> History {
    let mut history = History::new();
    let Ok(entries) = fs::read_dir(experiments) else {
        return history;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if id == current {
            continue;
        }
        let Ok(stored) = store::Stored::retained(&Native, experiments, &id) else {
            continue;
        };
        let Ok(results) = trial_results::terminal(&stored.directory) else {
            continue;
        };
        for result in results.iter().map(trial_results::Terminal::result) {
            let harness = match result.arm {
                Arm::Baseline => &stored.description.baseline.harness,
                Arm::Candidate => &stored.description.candidate.harness,
            };
            if let Some(minutes) = completed_minutes(result) {
                history
                    .entry((result.suite.clone(), harness.clone()))
                    .or_default()
                    .include(minutes);
            }
        }
    }
    history
}

/// The deciding attempt's minutes of a Trial that completed, not void.
fn completed_minutes(result: &TrialResult) -> Option<f64> {
    if result.outcome == Outcome::Void {
        return None;
    }
    let attempt = result.attempts.last()?;
    Some(attempt.timings.total_ms as f64 / 60_000.0)
}

struct Plan {
    id: String,
    total: usize,
    timeout_minutes: f64,
    groups: Vec<Group>,
}

/// The remaining Trials of one suite and Arm.
struct Group {
    suite: String,
    arm: Arm,
    harness: String,
    trials: usize,
    mean: Option<f64>,
}

impl Plan {
    /// `None` when every Trial already has a result.
    fn new(experiment: &store::Stored, history: &History) -> Option<Self> {
        let description = &experiment.description;
        let remaining = workers::remaining(experiment);
        if remaining.is_empty() {
            return None;
        }
        let mut groups = Vec::new();
        for suite in &description.suites {
            for (arm, setup) in [
                (Arm::Baseline, &description.baseline),
                (Arm::Candidate, &description.candidate),
            ] {
                let trials = remaining
                    .iter()
                    .filter(|trial| trial.task.suite == *suite && trial.arm == arm)
                    .count();
                if trials > 0 {
                    groups.push(Group {
                        suite: suite.clone(),
                        arm,
                        harness: setup.harness.clone(),
                        trials,
                        mean: history
                            .get(&(suite.clone(), setup.harness.clone()))
                            .map(|mean| mean.get()),
                    });
                }
            }
        }
        let total = experiment
            .tasks
            .iter()
            .map(|task| 2 * description.bar.trials_per_task[&task.suite])
            .sum();
        Some(Self {
            id: description.id.clone(),
            total,
            timeout_minutes: description.bar.timeout_seconds as f64 / 60.0,
            groups,
        })
    }

    fn remaining(&self) -> usize {
        self.groups.iter().map(|group| group.trials).sum()
    }

    fn lines(&self) -> Vec<String> {
        let remaining = self.remaining();
        let mut lines = vec![if remaining == self.total {
            format!("Experiment {}: {} to run", self.id, trials(remaining))
        } else {
            format!(
                "Experiment {}: {remaining} of {} Trials remain",
                self.id, self.total
            )
        }];
        for group in &self.groups {
            let arm = match group.arm {
                Arm::Baseline => "Baseline",
                Arm::Candidate => "Candidate",
            };
            let estimate = match group.mean {
                Some(mean) => format!(
                    "estimate {} session-minutes",
                    minutes(mean * group.trials as f64)
                ),
                None => "no earlier Trials to estimate from".into(),
            };
            lines.push(format!(
                "{}, {arm} arm ({}): {}, {estimate}",
                group.suite,
                group.harness,
                trials(group.trials)
            ));
        }
        lines.push(format!(
            "Worst case: {} session-minutes ({} × the Bar's {}-minute timeout)",
            minutes(remaining as f64 * self.timeout_minutes),
            trials(remaining),
            minutes(self.timeout_minutes)
        ));
        let estimates: Option<Vec<f64>> = self
            .groups
            .iter()
            .map(|group| group.mean.map(|mean| mean * group.trials as f64))
            .collect();
        if let Some(estimates) = estimates {
            lines.push(format!(
                "Estimate: {} session-minutes, from this machine's earlier Experiments",
                minutes(estimates.iter().sum())
            ));
        }
        lines
    }
}

fn trials(count: usize) -> String {
    if count == 1 {
        "1 Trial".into()
    } else {
        format!("{count} Trials")
    }
}

/// Minutes to one decimal place, without a trailing `.0`.
fn minutes(minutes: f64) -> String {
    let shown = format!("{minutes:.1}");
    shown.strip_suffix(".0").map_or(shown.clone(), String::from)
}
