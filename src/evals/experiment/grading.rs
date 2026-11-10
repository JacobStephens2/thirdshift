//! Read-only Experiment assessment: effective paired Scores, suite Verdicts and
//! whole-Experiment decisive rechecks, kept with private Trial data.

pub(super) mod process;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::deciders::{Call, Decided, Deciders};
use super::{Arm, Bar, Setup, Task, TrialResult};
use crate::sandbox::code_grader::{self, Check};
use crate::trial::{Outcome, Output};
use crate::verdict::{self, Decision, TaskScores};

#[derive(Serialize, Deserialize)]
pub(crate) struct Score {
    pub passed: bool,
    pub wrong_finding: bool,
    #[serde(default)]
    pub borderline: bool,
    #[serde(default)]
    pub model: Option<super::model_grader::Score>,
    pub checks: Vec<Check>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rechecks: Vec<super::rechecks::Recheck>,
}

impl Score {
    pub fn effective(&self) -> verdict::Score {
        let latest = self.rechecks.last();
        verdict::Score {
            passed: latest.map_or(self.passed, |recheck| recheck.passed),
            wrong_finding: latest
                .and_then(|recheck| recheck.wrong_finding)
                .unwrap_or(self.wrong_finding),
        }
    }

    pub fn with_model(&mut self, score: super::model_grader::Score) {
        self.passed = self.checks.iter().all(|check| check.passed) && score.passed;
        self.wrong_finding = score.wrong_finding;
        self.borderline = score.borderline;
        self.model = Some(score);
    }
}

pub(super) fn score(
    task: &Task,
    repository: &Path,
    output: &Output,
    timeout: Duration,
) -> Result<Score> {
    let mut checks = vec![Check {
        command: None,
        passed: matches!(output.outcome, Outcome::Succeeded | Outcome::Delivered),
        reason: format!(
            "Trial {}: {}",
            output.outcome,
            output.cause.as_deref().unwrap_or("session completed")
        ),
    }];
    if task.manifest.session != crate::evals::format::Session::Process {
        checks.push(Check {
            command: None,
            passed: output.reports.standards.is_some() || output.reports.spec.is_some(),
            reason: "review must leave at least one report".into(),
        });
    }
    if output.outcome != Outcome::Void
        && task.manifest.session == crate::evals::format::Session::Process
    {
        let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
        checks.extend(process::execute(task, repository, timeout, &caps)?);
    } else if output.outcome != Outcome::Void && !task.manifest.checks.is_empty() {
        let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
        checks.extend(code_grader::execute(
            repository,
            &output.reports,
            &task.manifest.checks,
            code_grader::Timeout::PerCheck(timeout),
            &caps,
            false,
        )?);
    }
    Ok(Score {
        passed: checks.iter().all(|check| check.passed),
        // Code checks cannot judge a written finding. Model Graders add that
        // evidence in their separate stage; do not infer it from report text.
        wrong_finding: false,
        borderline: false,
        model: None,
        checks,
        rechecks: Vec::new(),
    })
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Verdict {
    pub verdict: Conclusion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efficiency: Option<super::efficiency::Efficiency>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(default, skip_serializing_if = "Provisional::is_missing")]
    pub provisional_verdict: Provisional,
    pub suites: BTreeMap<String, SuiteVerdict>,
    #[serde(default)]
    borderline_trials: usize,
    /// Absent unless the Bar counts the panel's or the people's calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deciders: Option<Deciders>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value")]
pub(crate) struct SuiteVerdict {
    #[serde(flatten)]
    pub statistics: verdict::Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<Counts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement: Option<Vec<Measurement>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory: Option<bool>,
}

// Deserialize the statistical object directly: serde's flatten buffer turns
// arbitrary-precision JSON numbers into maps before the f64 fields see them.
impl TryFrom<serde_json::Value> for SuiteVerdict {
    type Error = serde_json::Error;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        Ok(Self {
            advisory: value
                .get("advisory")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            counts: value
                .get("counts")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            measurement: value
                .get("measurement")
                .cloned()
                .map(serde_json::from_value)
                .transpose()?,
            statistics: serde_json::from_value(value)?,
        })
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Counts {
    pub judged: usize,
    pub measurement_only: usize,
    pub void_dropped: usize,
    pub other_setup: usize,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Measurement {
    pub task: String,
    pub trials: Vec<MeasurementTrial>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct MeasurementTrial {
    pub arm: Arm,
    pub trial: usize,
    pub outcome: Outcome,
    pub passed: bool,
    pub wrong_finding: bool,
    pub borderline: bool,
}

/// Missing and explicit null are distinct facts in a saved legacy Verdict.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Provisional {
    Recorded(Option<Decision>),
    #[default]
    #[serde(skip)]
    Missing,
}

impl Provisional {
    fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    pub(crate) fn decision(self) -> Option<Decision> {
        match self {
            Self::Recorded(decision) => decision,
            Self::Missing => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Conclusion {
    Pass,
    Fail,
    Inconclusive,
    WaitingOnRechecks,
    WaitingOnLeaks,
    WaitingOnPeople,
}

impl From<Decision> for Conclusion {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Pass => Self::Pass,
            Decision::Fail => Self::Fail,
            Decision::Inconclusive => Self::Inconclusive,
        }
    }
}

impl Verdict {
    pub(super) fn update_line(&mut self) {
        let Some(efficiency) = &self.efficiency else {
            return;
        };
        let mut line = self.outcome().replace('_', " ");
        if matches!(self.verdict, Conclusion::Inconclusive)
            && let Some(reason) = self
                .deciders
                .as_ref()
                .and_then(|deciders| deciders.inconclusive.as_ref())
        {
            line.push_str(&format!(" ({reason})"));
        }
        if matches!(self.verdict, Conclusion::Pass) {
            line.push_str(if self.worse_within_margin_metrics().is_empty() {
                ": no worse"
            } else {
                ": worse, within the margin"
            });
        }
        let annotation = efficiency.annotation();
        if !annotation.is_empty() {
            line.push_str(if matches!(self.verdict, Conclusion::Pass) {
                ", and "
            } else {
                ": "
            });
            line.push_str(&annotation);
        }
        self.line = Some(line);
    }

    pub fn selection_report(&self) -> String {
        let mut report = String::new();
        for (suite, verdict) in &self.suites {
            if verdict.advisory == Some(true) {
                let decision = match verdict.statistics.verdict {
                    Decision::Pass => "pass",
                    Decision::Fail => "fail",
                    Decision::Inconclusive => "inconclusive",
                };
                report.push_str(&format!("\n{suite}: Advisory suite ({decision})"));
            }
            if let Some(counts) = &verdict.counts {
                report.push_str(&format!(
                    "\n{suite}: {} judged, {} measurement-only, {} void-dropped, {} other-setup",
                    counts.judged, counts.measurement_only, counts.void_dropped, counts.other_setup
                ));
            }
            for measurement in verdict.measurement.iter().flatten() {
                report.push_str(&format!("\nMeasurement {suite}/{}", measurement.task));
                for trial in &measurement.trials {
                    report.push_str(&format!(
                        "\n  {} Trial {}: {}, passed: {}, wrong finding: {}, borderline: {}",
                        trial.arm.name(),
                        trial.trial,
                        trial.outcome,
                        trial.passed,
                        trial.wrong_finding,
                        trial.borderline
                    ));
                }
            }
        }
        report
    }

    /// Passing metrics whose pass-level interval lies wholly below zero.
    pub fn worse_within_margin_metrics(&self) -> Vec<String> {
        self.suites
            .iter()
            .flat_map(|(suite, verdict)| {
                let verdict = &verdict.statistics;
                [
                    ("accuracy", &verdict.accuracy),
                    ("wrong findings", &verdict.wrong_findings),
                ]
                .into_iter()
                .filter(|(_, assessment)| assessment.worse_within_margin)
                .map(move |(metric, _)| format!("{suite} {metric}"))
            })
            .collect()
    }

    pub fn outcome(&self) -> &'static str {
        match self.verdict {
            Conclusion::Pass => "pass",
            Conclusion::Fail => "fail",
            Conclusion::Inconclusive => "inconclusive",
            Conclusion::WaitingOnRechecks => "waiting_on_rechecks",
            Conclusion::WaitingOnLeaks => "waiting_on_leaks",
            Conclusion::WaitingOnPeople => "waiting_on_people",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Pending {
    pub id: String,
    pub suite: String,
    pub task: String,
    pub arm: Arm,
    pub trial: usize,
    pub borderline: bool,
    pub decisive: bool,
    #[serde(default)]
    pub decisive_flips: Vec<Flip>,
    #[serde(default)]
    pub decisive_verdicts: BTreeMap<Flip, Decision>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Flip {
    Passed,
    WrongFinding,
}

impl std::fmt::Display for Pending {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}: {}/{} {} Trial {} ({})",
            self.id,
            self.suite,
            self.task,
            self.arm.name(),
            self.trial,
            match (self.borderline, self.decisive) {
                (true, true) => "borderline, decisive",
                (true, false) => "borderline",
                _ => "decisive",
            }
        )
    }
}

pub(super) fn task_is_void(result: &TrialResult, results: &[TrialResult]) -> bool {
    results.iter().any(|other| {
        other.suite == result.suite && other.task == result.task && other.outcome == Outcome::Void
    })
}

pub(super) fn assess(
    tasks: &[Task],
    results: &[TrialResult],
    bar: &Bar,
    baseline: &Setup,
    decided: &Decided,
) -> Result<(Verdict, Vec<Pending>)> {
    let mut verdict = evaluate(tasks, results, bar, baseline, decided, None)?;
    let original = verdict
        .provisional_verdict
        .decision()
        .expect("computed Verdict");
    let mut pending = Vec::new();
    for (index, result) in results.iter().enumerate() {
        // A Void Trial drops the whole paired task, including the other Arm.
        if bar.advisory(&result.suite) == Some(true)
            || !bar.judges(&result.suite, &result.task)
            || task_is_void(result, results)
        {
            continue;
        }
        let score = result.score.as_ref().context("Trial has no Score")?;
        if !score.rechecks.is_empty() {
            continue;
        }
        let mut decisive_flips = Vec::new();
        let mut decisive_verdicts = BTreeMap::new();
        for flip in [Flip::Passed, Flip::WrongFinding] {
            let flipped = evaluate(tasks, results, bar, baseline, decided, Some((index, flip)))?
                .provisional_verdict
                .decision()
                .expect("computed Verdict");
            if flipped != original {
                decisive_flips.push(flip);
                decisive_verdicts.insert(flip, flipped);
            }
        }
        let decisive = !decisive_flips.is_empty();
        if score.borderline || decisive {
            pending.push(Pending {
                id: result.key(),
                suite: result.suite.clone(),
                task: result.task.clone(),
                arm: result.arm,
                trial: result.trial,
                borderline: score.borderline,
                decisive,
                decisive_flips,
                decisive_verdicts,
            });
        }
    }
    if !pending.is_empty() {
        verdict.verdict = Conclusion::WaitingOnRechecks;
    }
    let efficiency = super::efficiency::Efficiency::of(results, bar, original);
    let decision = efficiency.decision();
    verdict.provisional_verdict = Provisional::Recorded(Some(decision));
    if pending.is_empty() {
        verdict.verdict = decision.into();
    }
    // Calls still to come from people change Scores, so they come first.
    if verdict
        .deciders
        .as_ref()
        .is_some_and(|deciders| deciders.waiting_pairs > 0)
    {
        verdict.verdict = Conclusion::WaitingOnPeople;
    }
    verdict.efficiency = Some(efficiency);
    verdict.update_line();
    Ok((verdict, pending))
}

fn evaluate(
    tasks: &[Task],
    results: &[TrialResult],
    bar: &Bar,
    baseline: &Setup,
    decided: &Decided,
    flip: Option<(usize, Flip)>,
) -> Result<Verdict> {
    let mut suites = BTreeMap::new();
    let mut total = 0;
    let mut dropped = 0;
    let mut deciders = decided.report.clone();
    for (suite, &trials) in &bar.trials_per_task {
        let advisory = bar.advisory(suite);
        // A Verdict on the people's sample alone takes each pair as a unit.
        let unit = if decided.sample_only { 1 } else { trials };
        let statistics = verdict::Bar {
            margin: bar.margin,
            levels: bar.levels,
            prior: bar.prior,
            seed: bar.seed,
            draws: bar.draws,
            trials_per_task: unit,
            ..verdict::Bar::default()
        };
        let mut scores = Vec::new();
        let mut counts = Counts::default();
        let mut measurement = Vec::new();
        for task in tasks.iter().filter(|task| &task.suite == suite) {
            let judged = bar.judges(suite, &task.name);
            let process = task.manifest.session == crate::evals::format::Session::Process;
            let mut measured_trials = Vec::new();
            let dropped_as_void = results.iter().any(|result| {
                result.suite == task.suite
                    && result.task == task.name
                    && result.outcome == Outcome::Void
            });
            // Each Trial's Score by its index, Baseline then Candidate.
            let mut by_trial: BTreeMap<usize, [Option<verdict::Score>; 2]> = BTreeMap::new();
            let mut left_out = std::collections::BTreeSet::new();
            for (index, result) in results
                .iter()
                .enumerate()
                .filter(|(_, result)| result.suite == task.suite && result.task == task.name)
            {
                let mut score = result
                    .score
                    .as_ref()
                    .context("Trial has no mechanical Score")?
                    .effective();
                if !judged {
                    measured_trials.push(MeasurementTrial {
                        arm: result.arm,
                        trial: result.trial,
                        outcome: result.outcome,
                        passed: score.passed,
                        wrong_finding: score.wrong_finding,
                        borderline: result.score.as_ref().is_some_and(|score| score.borderline),
                    });
                }
                if let Some((selected, field)) = flip
                    && selected == index
                {
                    match field {
                        Flip::Passed => score.passed = !score.passed,
                        Flip::WrongFinding => score.wrong_finding = !score.wrong_finding,
                    }
                }
                if judged && process && decided.report.is_some() {
                    let key = super::trial_results::pair_key(&task.suite, &task.name, result.trial);
                    match decided.calls.get(&key) {
                        // A Trial passes only if the code Grader passes and
                        // whoever decides its pair calls it ready.
                        Some(Call::Ready {
                            baseline,
                            candidate,
                            ..
                        }) => {
                            score.passed &= match result.arm {
                                Arm::Baseline => *baseline,
                                Arm::Candidate => *candidate,
                            };
                        }
                        // Provisionally on the code Grader alone.
                        Some(Call::Waiting) => {}
                        Some(Call::Unread) => ensure!(
                            dropped_as_void,
                            "the Grader panel has no call on pair {key}"
                        ),
                        None => {
                            left_out.insert(result.trial);
                        }
                    }
                }
                let slot = &mut by_trial.entry(result.trial).or_default()
                    [usize::from(result.arm == Arm::Candidate)];
                ensure!(slot.replace(score).is_none(), "duplicate Trial result");
            }
            ensure!(
                by_trial.len() == trials
                    && by_trial
                        .values()
                        .all(|pair| pair.iter().all(Option::is_some)),
                "missing Trial results"
            );
            counts.void_dropped += usize::from(dropped_as_void);
            if !judged {
                counts.measurement_only += usize::from(!dropped_as_void);
                measurement.push(Measurement {
                    task: task.name.clone(),
                    trials: measured_trials,
                });
                continue;
            }
            if !dropped_as_void {
                counts.judged += 1;
                counts.other_setup +=
                    usize::from(task.manifest.difficulty.as_ref().is_some_and(|difficulty| {
                        difficulty.commit != baseline.commit
                            || difficulty.harness != baseline.harness
                            || difficulty.model != baseline.model
                            || difficulty.effort != baseline.effort
                    }));
                if let Some(report) = deciders.as_mut().filter(|_| process) {
                    for trial in 0..trials {
                        let key = super::trial_results::pair_key(&task.suite, &task.name, trial);
                        match decided.calls.get(&key) {
                            Some(Call::Ready { people: true, .. }) => report.people_pairs += 1,
                            Some(Call::Ready { people: false, .. }) => report.panel_pairs += 1,
                            Some(Call::Waiting) => report.waiting_pairs += 1,
                            Some(Call::Unread) => {}
                            None => report.left_out_pairs += 1,
                        }
                    }
                }
            }
            if advisory != Some(true) {
                dropped += usize::from(dropped_as_void);
                total += 1;
            }
            let mut paired = TaskScores {
                baseline: Vec::new(),
                candidate: Vec::new(),
                dropped_as_void,
            };
            for (trial, [baseline, candidate]) in by_trial {
                if left_out.contains(&trial) {
                    continue;
                }
                paired.baseline.push(baseline.expect("checked Trial pair"));
                paired
                    .candidate
                    .push(candidate.expect("checked Trial pair"));
                if decided.sample_only {
                    scores.push(std::mem::replace(
                        &mut paired,
                        TaskScores {
                            baseline: Vec::new(),
                            candidate: Vec::new(),
                            dropped_as_void,
                        },
                    ));
                }
            }
            if !decided.sample_only {
                scores.push(paired);
            }
        }
        // Counterfactuals need the seeded interval decision, never forecasts.
        let mut assessment = if flip.is_none() {
            verdict::judge(&scores, &statistics)?
        } else {
            verdict::judge_without_predictions(&scores, &statistics)?
        };
        let selected = bar.discriminating_tasks.is_some();
        if selected {
            assessment.method = "paired-dirichlet-v4; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles".into();
        }
        suites.insert(
            suite.clone(),
            SuiteVerdict {
                statistics: assessment,
                counts: selected.then_some(counts),
                measurement: selected.then_some(measurement),
                advisory,
            },
        );
    }
    let deciding: Vec<_> = suites
        .values()
        .filter(|suite| suite.advisory != Some(true))
        .collect();
    let low_agreement = deciders
        .as_ref()
        .is_some_and(|deciders| deciders.inconclusive.is_some());
    let verdict = if deciding.is_empty() || dropped > total / 10 || low_agreement {
        Decision::Inconclusive
    } else if deciding
        .iter()
        .any(|suite| suite.statistics.verdict == Decision::Fail)
    {
        Decision::Fail
    } else if deciding
        .iter()
        .any(|suite| suite.statistics.verdict == Decision::Inconclusive)
    {
        Decision::Inconclusive
    } else {
        Decision::Pass
    };
    let borderline_trials = results
        .iter()
        .filter(|result| result.score.as_ref().is_some_and(|score| score.borderline))
        .count();
    Ok(Verdict {
        verdict: verdict.into(),
        efficiency: None,
        line: None,
        provisional_verdict: Provisional::Recorded(Some(verdict)),
        suites,
        borderline_trials,
        deciders,
    })
}

#[cfg(test)]
mod tests;
