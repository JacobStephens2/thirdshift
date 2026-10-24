//! Human decisions supplement immutable Grader Scores. One locked metadata
//! write publishes their history, recalculated Verdict and pending list together.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::grading::{self, Conclusion, Verdict};
use super::store::{State, Stored};
use super::workers::{Arm, TrialResult};
use super::{Bar, Task};
use crate::trial::Outcome;

pub(super) type History = BTreeMap<String, Vec<Recheck>>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Recheck {
    pub passed: bool,
    pub reason: String,
    pub wrong_finding: Option<bool>,
    pub recorded_at: String,
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

#[derive(Clone, Serialize, Deserialize)]
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

fn task_is_void(result: &TrialResult, results: &[TrialResult]) -> bool {
    results.iter().any(|other| {
        other.suite == result.suite && other.task == result.task && other.outcome == Outcome::Void
    })
}

fn flip(score: &mut grading::Score, wrong_finding: bool) {
    let value = if wrong_finding {
        &mut score.wrong_finding
    } else {
        &mut score.passed
    };
    *value = !*value;
}

pub(super) fn assess(
    tasks: &[Task],
    results: &mut [TrialResult],
    bar: &Bar,
) -> Result<(Verdict, Vec<Pending>)> {
    let mut verdict = grading::judge(tasks, results, bar)?;
    let original = verdict
        .provisional_verdict
        .decision()
        .expect("computed Verdict");
    let mut pending = Vec::new();
    for index in 0..results.len() {
        let result = &results[index];
        // A Void Trial drops the whole paired task, including the other Arm.
        if task_is_void(result, results) {
            continue;
        }
        let score = result.score.as_ref().context("Trial has no Score")?;
        if !score.rechecks.is_empty() {
            continue;
        }
        let borderline = score.borderline;
        let mut decisive = false;
        let mut decisive_flips = Vec::new();
        for wrong_finding in [false, true] {
            let score = results[index]
                .score
                .as_mut()
                .context("Trial has no Score")?;
            flip(score, wrong_finding);
            let flipped = grading::decision(tasks, results, bar);
            let score = results[index]
                .score
                .as_mut()
                .context("Trial has no Score")?;
            flip(score, wrong_finding);
            if flipped? != original {
                decisive = true;
                decisive_flips.push(if wrong_finding {
                    Flip::WrongFinding
                } else {
                    Flip::Passed
                });
            }
        }
        if borderline || decisive {
            let result = &results[index];
            pending.push(Pending {
                id: result.key(),
                suite: result.suite.clone(),
                task: result.task.clone(),
                arm: result.arm,
                trial: result.trial,
                borderline,
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

pub(crate) fn record(args: &[String]) -> Result<String> {
    let usage = "usage: thirdshift recheck <experiment-id> <trial-id> <pass|fail> --reason <text> [--wrong-finding <true|false>] [--json]";
    let [id, trial, decision, rest @ ..] = args else {
        bail!("{usage}")
    };
    let passed = match decision.as_str() {
        "pass" => true,
        "fail" => false,
        _ => bail!("{usage}"),
    };
    let mut reason = None;
    let mut wrong_finding = None;
    let mut json = false;
    let mut flags = rest.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--reason" if reason.is_none() => reason = flags.next().cloned(),
            "--wrong-finding" if wrong_finding.is_none() => {
                wrong_finding = Some(
                    flags
                        .next()
                        .context(usage)?
                        .parse::<bool>()
                        .context(usage)?,
                );
            }
            "--json" if !json => json = true,
            _ => bail!("{usage}"),
        }
    }
    let reason = reason
        .filter(|reason| !reason.trim().is_empty())
        .context("a nonempty --reason is required")?;
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    let mut experiment = Stored::load(&root, id, true)?;
    ensure!(
        experiment.state == State::Completed,
        "Experiment must finish grading before rechecks"
    );
    let results = experiment.results()?;
    let result = results
        .iter()
        .find(|result| result.key() == *trial)
        .context("Trial not found")?;
    ensure!(
        !task_is_void(result, &results),
        "Void Eval tasks cannot be rechecked"
    );
    ensure!(result.score.is_some(), "Trial has no Score");
    // Omission retains the previous human flag, or the Grader's on the first recheck.
    let wrong_finding =
        wrong_finding.or_else(|| result.score.as_ref()?.rechecks.last()?.wrong_finding);
    experiment
        .rechecks
        .entry(trial.clone())
        .or_default()
        .push(Recheck {
            passed,
            reason,
            wrong_finding,
            recorded_at: chrono::Local::now().to_rfc3339(),
        });
    experiment.refresh_verdict()?;
    experiment.save()?;
    if let Some(log) = &experiment.details.command_log
        && let Err(error) = crate::logs::experiment_rechecked(
            log,
            &experiment.description.id,
            experiment
                .verdict
                .as_ref()
                .expect("rechecked Verdict")
                .outcome(),
        )
    {
        crate::progress::step(format!(
            "warning: could not update the Command record: {error:#}"
        ));
    }
    experiment.render(json)
}
