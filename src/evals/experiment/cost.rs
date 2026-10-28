//! What an Experiment's remaining Trials may cost, shown before the first of
//! them runs. A terminal confirms it; `--yes` and piped stdin go ahead.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::trial_results::files::{self, Files, Native};
use super::{Arm, TrialResult, store, trial_results, workers};
use crate::progress;
use crate::trial::Outcome;

const QUESTION: &str = "Run these Trials? [y/N] ";

/// Show the remaining Trials' cost, then ask on a terminal unless `yes`.
/// Declining returns an error before any Arm is built.
pub(super) fn confirm(experiment: &store::Stored, yes: bool) -> Result<()> {
    let experiments = experiment
        .directory
        .parent()
        .context("missing Experiments folder")?;
    let Some(plan) = Plan::new(
        experiment,
        &history(&Native, experiments, &experiment.description.id),
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

/// Completed Trials' minutes, by suite and Harness.
type History = BTreeMap<(String, String), TrialMinutes>;

#[derive(Clone, Copy, Default)]
struct TrialMinutes {
    total: f64,
    trials: usize,
}

impl TrialMinutes {
    fn add(&mut self, minutes: f64) {
        self.total += minutes;
        self.trials += 1;
    }

    fn per_trial(self) -> f64 {
        self.total / self.trials as f64
    }
}

/// Completed, non-void Trials of this machine's other Experiments. An
/// unreadable Experiment only narrows the estimate, so it is left out.
fn history(files: &impl Files, experiments: &Path, current: &str) -> History {
    let mut history = History::new();
    let Ok(entries) = files::directory(files, experiments) else {
        return history;
    };
    for entry in entries {
        let Some(id) = entry.path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if id == current {
            continue;
        }
        let Ok(stored) = store::Stored::retained(files, experiments, id) else {
            continue;
        };
        let Ok(results) = trial_results::read_terminal(files, &stored.directory) else {
            continue;
        };
        for result in results.iter().map(trial_results::Terminal::result) {
            if let Some(minutes) = completed_minutes(result) {
                let harness = &stored.description.setup(result.arm).harness;
                history
                    .entry((result.suite.clone(), harness.clone()))
                    .or_default()
                    .add(minutes);
            }
        }
    }
    history
}

/// Every attempt's minutes of a Trial that completed, not void, as the
/// Experiment's own Arm minutes count them.
fn completed_minutes(result: &TrialResult) -> Option<f64> {
    (result.outcome != Outcome::Void).then(|| {
        result
            .attempts
            .iter()
            .map(|attempt| attempt.timings.total_ms as f64 / 60_000.0)
            .sum()
    })
}

struct Plan {
    id: String,
    /// Every Trial the Experiment plans, finished or not.
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
            for arm in [Arm::Baseline, Arm::Candidate] {
                let setup = description.setup(arm);
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
                            .map(|minutes| minutes.per_trial()),
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
    match shown.strip_suffix(".0") {
        Some(whole) => whole.to_owned(),
        None => shown,
    }
}
