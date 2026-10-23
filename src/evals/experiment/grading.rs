//! Mechanical Scores and paired suite Verdicts, kept with private Trial data.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
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
    pub checks: Vec<Check>,
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
        for command in &task.manifest.checks {
            let mut child = Command::new(&command[0]);
            child.args(&command[1..]).current_dir(repository);
            for (name, path) in [
                (
                    "THIRDSHIFT_TRIAL_STANDARDS_REPORT",
                    &output.reports.standards,
                ),
                ("THIRDSHIFT_TRIAL_SPEC_REPORT", &output.reports.spec),
                ("THIRDSHIFT_TRIAL_LOG", &output.log),
            ] {
                child.env(name, path.as_deref().unwrap_or(Path::new("")));
            }
            let result = process::output_until(
                &mut child,
                None,
                Control {
                    name: "code Grader",
                    interruption: Interruption::Ordinary,
                    stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
                },
                Instant::now() + Duration::from_secs(bar.timeout_seconds),
            );
            crate::interrupt::check()?;
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
        checks,
    })
}

#[derive(Serialize, Deserialize)]
pub(super) struct Verdict {
    pub verdict: Decision,
    suites: BTreeMap<String, verdict::Verdict>,
}

impl Verdict {
    pub fn outcome(&self) -> &'static str {
        match self.verdict {
            Decision::Pass => "pass",
            Decision::Fail => "fail",
            Decision::Inconclusive => "inconclusive",
        }
    }
}

pub(super) fn judge(tasks: &[Task], results: &[TrialResult], bar: &Bar) -> Result<Verdict> {
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
                arm.push(verdict::Score {
                    passed: score.passed,
                    wrong_finding: score.wrong_finding,
                });
            }
            ensure!(
                paired.baseline.len() == trials && paired.candidate.len() == trials,
                "missing Trial results"
            );
            dropped += usize::from(paired.dropped_as_void);
            total += 1;
            scores.push(paired);
        }
        suites.insert(suite.clone(), verdict::judge(&scores, &statistics)?);
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
    Ok(Verdict { verdict, suites })
}
