//! Combined Scores and paired suite Verdicts, kept with private Trial data.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::workers::{Arm, TrialResult};
use super::{Bar, Task};
use crate::process::{self, Control, Interruption};
use crate::trial::{Outcome, Output};
use crate::verdict::{self, Decision, TaskScores};

#[derive(Serialize, Deserialize)]
pub(super) struct Score {
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

#[derive(Serialize, Deserialize)]
pub(super) struct Check {
    command: Option<Vec<String>>,
    passed: bool,
    reason: String,
}

pub(super) fn score(task: &Task, repository: &Path, output: &Output, bar: &Bar) -> Result<Score> {
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
    if output.outcome != Outcome::Void {
        let packet = tempfile::Builder::new().prefix("grader-").tempdir()?;
        for (source, name) in [
            (&output.reports.standards, "standards.md"),
            (&output.reports.spec, "spec.md"),
        ] {
            if let Some(source) = source {
                fs::write(
                    packet.path().join(name),
                    super::model_grader::blind(&fs::read_to_string(source)?),
                )?;
            }
        }
        for command in &task.manifest.checks {
            let mut child = crate::sandbox::code_grader(repository, packet.path(), command)?;
            let result = process::output_until(
                &mut child,
                None,
                Control {
                    name: "code Grader",
                    interruption: Interruption::Ordinary,
                    stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
                },
                Instant::now() + Duration::from_secs(bar.timeout_seconds),
                &|| Ok(()),
            );
            crate::interrupt::check()?;
            if let Ok(output) = &result {
                crate::sandbox::check_started(output)?;
            }
            let (passed, reason) = match result {
                Ok(result) => (
                    result.status.success(),
                    format!(
                        "{}\n{}{}",
                        result.status,
                        String::from_utf8_lossy(&result.stdout),
                        String::from_utf8_lossy(&result.stderr)
                    ),
                ),
                Err(error) => (false, format!("{error:#}")),
            };
            checks.push(Check {
                command: Some(command.clone()),
                passed,
                reason,
            });
        }
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

#[derive(Serialize, Deserialize)]
pub(super) struct Verdict {
    pub verdict: Conclusion,
    #[serde(default)]
    pub provisional_verdict: Option<Decision>,
    suites: BTreeMap<String, verdict::Verdict>,
    #[serde(default)]
    borderline_trials: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Conclusion {
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

pub(super) fn judge(tasks: &[Task], results: &[TrialResult], bar: &Bar) -> Result<Verdict> {
    evaluate(tasks, results, bar, true)
}

pub(super) fn decision(tasks: &[Task], results: &[TrialResult], bar: &Bar) -> Result<Decision> {
    Ok(evaluate(tasks, results, bar, false)?
        .provisional_verdict
        .expect("a computed Verdict has a provisional decision"))
}

fn evaluate(
    tasks: &[Task],
    results: &[TrialResult],
    bar: &Bar,
    predictions: bool,
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
            for result in results
                .iter()
                .filter(|result| result.suite == task.suite && result.task == task.name)
            {
                paired.dropped_as_void |= result.outcome == Outcome::Void;
                let score = result
                    .score
                    .as_ref()
                    .context("Trial has no mechanical Score")?;
                let arm = match result.arm {
                    Arm::Baseline => &mut paired.baseline,
                    Arm::Candidate => &mut paired.candidate,
                };
                arm.push(score.effective());
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
        let assessment = if predictions {
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
