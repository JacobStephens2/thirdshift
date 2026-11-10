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
use crate::trial::accounting::Prices;

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

/// Completed Trials' measurements, by suite and exact Arm setup.
type History = BTreeMap<(String, String), TrialMeasurements>;

#[derive(Clone, Copy, Default)]
struct TrialMeasurements {
    total_minutes: f64,
    trials: usize,
    cost_usd: f64,
    priced_trials: usize,
}

impl TrialMeasurements {
    fn add(&mut self, minutes: f64, cost_usd: Option<f64>) {
        self.total_minutes += minutes;
        self.trials += 1;
        if let Some(cost) = cost_usd {
            self.cost_usd += cost;
            self.priced_trials += 1;
        }
    }

    fn minutes_per_trial(self) -> f64 {
        self.total_minutes / self.trials as f64
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
                let setup = stored.description.setup(result.arm);
                history
                    .entry(history_key(&result.suite, setup))
                    .or_default()
                    .add(minutes, super::accounting::trial_total(result).cost_usd);
            }
        }
    }
    history
}

fn history_key(suite: &str, setup: &super::Setup) -> (String, String) {
    (
        suite.into(),
        serde_json::to_string(setup).expect("Arm setup JSON"),
    )
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
    seats: Vec<Seat>,
    spending_limit: Option<f64>,
}

/// The remaining sessions of one Grader panel seat.
struct Seat {
    name: String,
    model: String,
    sessions: usize,
    pairs: usize,
    /// Per session, with how it was reached.
    price: Option<(f64, &'static str)>,
}

/// The research's assumed pairwise Grader session, until one is measured:
/// 400k input tokens, 80% of them cache reads, and 15k output tokens.
const ASSUMED_SESSION: (f64, f64, f64) = (80_000.0, 320_000.0, 15_000.0);

fn assumed_session(prices: Option<&Prices>, model: &str) -> Option<f64> {
    let rate = prices?.models.get(model)?;
    let (input, cache_read, output) = ASSUMED_SESSION;
    Some((input * rate.input + cache_read * rate.cache_read + output * rate.output) / 1_000_000.0)
}

/// The remaining Trials of one suite and Arm.
struct Group {
    suite: String,
    arm: Arm,
    harness: String,
    trials: usize,
    mean: Option<f64>,
    mean_cost: Option<f64>,
}

impl Plan {
    /// `None` when every Trial already has a result and the Grader panel
    /// has made every call.
    fn new(experiment: &store::Stored, history: &History) -> Option<Self> {
        let description = &experiment.description;
        let remaining = workers::remaining(experiment);
        let seats = seats(experiment);
        if remaining.is_empty() && seats.is_empty() {
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
                            .get(&history_key(suite, setup))
                            .map(|measurements| measurements.minutes_per_trial()),
                        mean_cost: history
                            .get(&history_key(suite, setup))
                            .filter(|past| past.priced_trials == past.trials)
                            .map(|past| past.cost_usd / past.trials as f64),
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
            seats,
            spending_limit: description
                .bar
                .panel
                .as_ref()
                .and_then(|panel| panel.spending_limit),
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
            let estimate = match group.mean_cost {
                Some(cost) => format!("{estimate}, ${:.6}", cost * group.trials as f64),
                None => estimate,
            };
            lines.push(format!(
                "{}, {arm} arm ({}): {}, {estimate}",
                group.suite,
                group.harness,
                trials(group.trials)
            ));
        }
        if remaining > 0 {
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
        }
        for (index, seat) in self.seats.iter().enumerate() {
            let sessions = if seat.sessions == 2 * seat.pairs {
                let pairs = if seat.pairs == 1 { "pair" } else { "pairs" };
                format!(
                    "{} sessions ({} {pairs} × 2 orders)",
                    seat.sessions, seat.pairs
                )
            } else {
                format!("{} sessions", seat.sessions)
            };
            let estimate = match seat.price {
                Some((price, basis)) => format!(
                    "estimate ${:.6} at ${price:.6} per {basis}",
                    price * seat.sessions as f64
                ),
                None => format!("no frozen price for Model {}", seat.model),
            };
            lines.push(format!(
                "Grader panel seat {} ({}): {sessions}, {estimate}",
                index + 1,
                seat.name
            ));
        }
        if !self.seats.is_empty() {
            let priced: f64 = self
                .seats
                .iter()
                .filter_map(|seat| seat.price.map(|(price, _)| price * seat.sessions as f64))
                .sum();
            lines.push(format!(
                "Grader panel estimate: ${priced:.6} for priced seats, apart from the Arms"
            ));
        }
        if let Some(limit) = self.spending_limit {
            lines.push(format!(
                "Grader panel spending limit: ${limit:.6} per run, then grading pauses"
            ));
        }
        lines
    }
}

/// Each panel seat with sessions still to run, priced from the frozen table:
/// at this Experiment's measured mean once its sessions are all priced,
/// otherwise at the assumed session size.
fn seats(experiment: &store::Stored) -> Vec<Seat> {
    let Some(panel) = &experiment.description.bar.panel else {
        return Vec::new();
    };
    let prices = experiment.pricing.as_ref();
    let spent = super::panel::spend(
        &Native,
        &experiment.directory,
        &experiment.description,
        prices,
    )
    .ok()
    .flatten();
    let remaining = super::panel::remaining(experiment);
    if remaining.iter().all(|(sessions, _)| *sessions == 0) {
        return Vec::new();
    }
    panel
        .seats
        .iter()
        .zip(remaining)
        .enumerate()
        .map(|(index, (seat, (sessions, pairs)))| {
            let measured = spent
                .as_ref()
                .map(|spent| &spent.seats[index])
                .filter(|spent| spent.sessions > 0)
                .and_then(|spent| spent.cost_usd.map(|cost| cost / spent.sessions as f64));
            Seat {
                name: format!("{}/{}", seat.harness, seat.model),
                model: seat.model.clone(),
                sessions,
                pairs,
                price: measured
                    .map(|price| (price, "session measured in this Experiment"))
                    .or_else(|| {
                        assumed_session(prices, &seat.model)
                            .map(|price| (price, "assumed 400k-input, 15k-output session"))
                    }),
            }
        })
        .collect()
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
