//! Read-only Experiment assessment: effective paired Scores, suite Verdicts and
//! whole-Experiment decisive rechecks, kept with private Trial data.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

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
    let mut checks = vec![
        Check {
            command: None,
            passed: output.outcome == Outcome::Succeeded,
            reason: format!(
                "Trial {}: {}",
                output.outcome,
                output.cause.as_deref().unwrap_or("session completed")
            ),
        },
        Check {
            command: None,
            passed: output.reports.standards.is_some() || output.reports.spec.is_some(),
            reason: "review must leave at least one report".into(),
        },
    ];
    if output.outcome != Outcome::Void && !task.manifest.checks.is_empty() {
        let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
        checks.extend(code_grader::execute(
            repository,
            &output.reports,
            &task.manifest.checks,
            timeout,
            &caps,
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
    #[serde(default, skip_serializing_if = "Provisional::is_missing")]
    pub provisional_verdict: Provisional,
    pub suites: BTreeMap<String, SuiteVerdict>,
    #[serde(default)]
    borderline_trials: usize,
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
}

// Deserialize the statistical object directly: serde's flatten buffer turns
// arbitrary-precision JSON numbers into maps before the f64 fields see them.
impl TryFrom<serde_json::Value> for SuiteVerdict {
    type Error = serde_json::Error;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        Ok(Self {
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
    pub fn selection_report(&self) -> String {
        let mut report = String::new();
        for (suite, verdict) in &self.suites {
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
) -> Result<(Verdict, Vec<Pending>)> {
    let mut verdict = evaluate(tasks, results, bar, baseline, None)?;
    let original = verdict
        .provisional_verdict
        .decision()
        .expect("computed Verdict");
    let mut pending = Vec::new();
    for (index, result) in results.iter().enumerate() {
        // A Void Trial drops the whole paired task, including the other Arm.
        if !bar.judges(&result.suite, &result.task) || task_is_void(result, results) {
            continue;
        }
        let score = result.score.as_ref().context("Trial has no Score")?;
        if !score.rechecks.is_empty() {
            continue;
        }
        let mut decisive_flips = Vec::new();
        let mut decisive_verdicts = BTreeMap::new();
        for flip in [Flip::Passed, Flip::WrongFinding] {
            let flipped = evaluate(tasks, results, bar, baseline, Some((index, flip)))?
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
    Ok((verdict, pending))
}

fn evaluate(
    tasks: &[Task],
    results: &[TrialResult],
    bar: &Bar,
    baseline: &Setup,
    flip: Option<(usize, Flip)>,
) -> Result<Verdict> {
    let mut suites = BTreeMap::new();
    let mut total = 0;
    let mut dropped = 0;
    for (suite, &trials) in &bar.trials_per_task {
        let statistics = verdict::Bar {
            margin: bar.margin,
            levels: bar.levels,
            prior: bar.prior,
            seed: bar.seed,
            draws: bar.draws,
            trials_per_task: trials,
            ..verdict::Bar::default()
        };
        let mut scores = Vec::new();
        let mut counts = Counts::default();
        let mut measurement = Vec::new();
        for task in tasks.iter().filter(|task| &task.suite == suite) {
            let judged = bar.judges(suite, &task.name);
            let mut measured_trials = Vec::new();
            let mut paired = TaskScores {
                baseline: Vec::new(),
                candidate: Vec::new(),
                dropped_as_void: false,
            };
            for (index, result) in results
                .iter()
                .enumerate()
                .filter(|(_, result)| result.suite == task.suite && result.task == task.name)
            {
                paired.dropped_as_void |= result.outcome == Outcome::Void;
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
                let arm = match result.arm {
                    Arm::Baseline => &mut paired.baseline,
                    Arm::Candidate => &mut paired.candidate,
                };
                arm.push(score);
            }
            ensure!(
                paired.baseline.len() == trials && paired.candidate.len() == trials,
                "missing Trial results"
            );
            counts.void_dropped += usize::from(paired.dropped_as_void);
            if !judged {
                counts.measurement_only += usize::from(!paired.dropped_as_void);
                measurement.push(Measurement {
                    task: task.name.clone(),
                    trials: measured_trials,
                });
                continue;
            }
            if !paired.dropped_as_void {
                counts.judged += 1;
                counts.other_setup +=
                    usize::from(task.manifest.difficulty.as_ref().is_some_and(|difficulty| {
                        difficulty.commit != baseline.commit
                            || difficulty.harness != baseline.harness
                            || difficulty.model != baseline.model
                            || difficulty.effort != baseline.effort
                    }));
            }
            dropped += usize::from(paired.dropped_as_void);
            total += 1;
            scores.push(paired);
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
            },
        );
    }
    let verdict = if dropped > total / 10 {
        Decision::Inconclusive
    } else if suites
        .values()
        .any(|suite| suite.statistics.verdict == Decision::Fail)
    {
        Decision::Fail
    } else if suites
        .values()
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
        provisional_verdict: Provisional::Recorded(Some(verdict)),
        suites,
        borderline_trials,
    })
}

#[cfg(test)]
mod tests;
