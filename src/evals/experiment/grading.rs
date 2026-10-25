//! Read-only Experiment assessment: effective paired Scores, suite Verdicts and
//! whole-Experiment decisive rechecks, kept with private Trial data.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::{Arm, Bar, Task, TrialResult};
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
    #[serde(default)]
    pub provisional_verdict: Option<Decision>,
    pub suites: BTreeMap<String, verdict::Verdict>,
    #[serde(default)]
    borderline_trials: usize,
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
}

#[derive(Clone, Copy, Serialize, Deserialize)]
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
) -> Result<(Verdict, Vec<Pending>)> {
    let mut verdict = evaluate(tasks, results, bar, None)?;
    let original = verdict.provisional_verdict.expect("computed Verdict");
    let mut pending = Vec::new();
    for (index, result) in results.iter().enumerate() {
        // A Void Trial drops the whole paired task, including the other Arm.
        if task_is_void(result, results) {
            continue;
        }
        let score = result.score.as_ref().context("Trial has no Score")?;
        if !score.rechecks.is_empty() {
            continue;
        }
        let mut decisive_flips = Vec::new();
        for flip in [Flip::Passed, Flip::WrongFinding] {
            let flipped = evaluate(tasks, results, bar, Some((index, flip)))?;
            if flipped.provisional_verdict != Some(original) {
                decisive_flips.push(flip);
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
    flip: Option<(usize, Flip)>,
) -> Result<Verdict> {
    let mut suites = BTreeMap::new();
    let mut total = 0;
    let mut dropped = 0;
    for (suite, &trials) in &bar.trials_per_task {
        let statistics = verdict::Bar {
            margin: bar.margin,
            credible_level: bar.credible_level,
            prior: bar.prior,
            seed: bar.seed,
            draws: bar.draws,
            trials_per_task: trials,
            ..verdict::Bar::default()
        };
        let mut scores = Vec::new();
        for task in tasks.iter().filter(|task| &task.suite == suite) {
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
            dropped += usize::from(paired.dropped_as_void);
            total += 1;
            scores.push(paired);
        }
        // Counterfactuals need the seeded interval decision, never forecasts.
        let assessment = if flip.is_none() {
            verdict::judge(&scores, &statistics)?
        } else {
            verdict::judge_without_predictions(&scores, &statistics)?
        };
        suites.insert(suite.clone(), assessment);
    }
    let verdict = if dropped > total / 10 {
        Decision::Inconclusive
    } else if suites.values().any(|suite| suite.verdict == Decision::Fail) {
        Decision::Fail
    } else if suites
        .values()
        .any(|suite| suite.verdict == Decision::Inconclusive)
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
        provisional_verdict: Some(verdict),
        suites,
        borderline_trials,
    })
}

#[cfg(test)]
mod tests;
