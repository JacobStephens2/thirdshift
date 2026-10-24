//! Human decisions supplement immutable Grader Scores. One locked metadata
//! write publishes their history, recalculated Verdict and pending list together.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::grading::{self, Conclusion, Verdict};
use super::store::{State, Stored};
use super::workers::{Arm, TrialResult};
use super::{Bar, Task};
use crate::trial::Outcome;
use crate::verdict::Decision;

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
        let mut decisive_verdicts = BTreeMap::new();
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
            let flipped = flipped?;
            if flipped != original {
                decisive = true;
                let flag = if wrong_finding {
                    Flip::WrongFinding
                } else {
                    Flip::Passed
                };
                decisive_flips.push(flag);
                decisive_verdicts.insert(flag, flipped);
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
                decisive_verdicts,
            });
        }
    }
    if !pending.is_empty() {
        verdict.verdict = Conclusion::WaitingOnRechecks;
    }
    Ok((verdict, pending))
}

enum Target<'a> {
    Trial(&'a str),
    Batch(&'a str),
}

#[derive(Serialize)]
struct PendingList<'a> {
    experiment: &'a str,
    fingerprint: String,
    pending_rechecks: &'a [Pending],
}

impl<'a> PendingList<'a> {
    fn new(experiment: &'a Stored) -> Result<Self> {
        let id = &experiment.description.id;
        let pending = &experiment.pending_rechecks;
        Ok(Self {
            experiment: id,
            fingerprint: format!("{:x}", Sha256::digest(serde_json::to_vec(&(id, pending))?)),
            pending_rechecks: pending,
        })
    }
}

fn load_completed(id: &str) -> Result<Stored> {
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    let experiment = Stored::load(&root, id, true)?;
    ensure!(
        experiment.state == State::Completed,
        "Experiment must finish grading before rechecks"
    );
    Ok(experiment)
}

pub(crate) fn record(args: &[String]) -> Result<String> {
    let usage = "usage: thirdshift recheck <experiment-id> <trial-id> <pass|fail> --reason <text> [--wrong-finding <true|false>] [--json]\n       thirdshift recheck <experiment-id> --pending [--json]\n       thirdshift recheck <experiment-id> --batch <fingerprint> <pass|fail> --reason <text> [--wrong-finding <true|false>] [--json]";
    if let [id, flag, rest @ ..] = args
        && flag == "--pending"
    {
        let json = match rest {
            [] => false,
            [flag] if flag == "--json" => true,
            _ => bail!("{usage}"),
        };
        let mut experiment = load_completed(id)?;
        experiment.refresh_verdict()?;
        let list = PendingList::new(&experiment)?;
        return if json {
            Ok(serde_json::to_string(&list)?)
        } else {
            Ok(format!(
                "Experiment: {}\nFingerprint: {}{}",
                list.experiment,
                list.fingerprint,
                list.pending_rechecks
                    .iter()
                    .map(|pending| format!("\nRecheck: {pending}"))
                    .collect::<String>()
            ))
        };
    }
    let (id, target, decision, rest) = match args {
        [id, flag, fingerprint, decision, rest @ ..] if flag == "--batch" => {
            (id, Target::Batch(fingerprint), decision, rest)
        }
        [id, trial, decision, rest @ ..] if !trial.starts_with("--") => {
            (id, Target::Trial(trial), decision, rest)
        }
        _ => bail!("{usage}"),
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
    let mut experiment = load_completed(id)?;
    let trials = match target {
        Target::Trial(trial) => vec![trial.to_owned()],
        Target::Batch(fingerprint) => {
            experiment.refresh_verdict()?;
            let list = PendingList::new(&experiment)?;
            ensure!(
                list.fingerprint == fingerprint,
                "pending recheck list has changed (stale fingerprint); run thirdshift recheck {id} --pending and review it again"
            );
            ensure!(
                !list.pending_rechecks.is_empty(),
                "no pending Scores to recheck"
            );
            list.pending_rechecks
                .iter()
                .map(|pending| pending.id.clone())
                .collect()
        }
    };
    let results = experiment.results()?;
    let recorded_at = chrono::Local::now().to_rfc3339();
    for trial in trials {
        let result = results
            .iter()
            .find(|result| result.key() == trial)
            .context("Trial not found")?;
        ensure!(
            !task_is_void(result, &results),
            "Void Eval tasks cannot be rechecked"
        );
        ensure!(result.score.is_some(), "Trial has no Score");
        // Omission retains the previous human flag, or the Grader's on the first recheck.
        let wrong_finding =
            wrong_finding.or_else(|| result.score.as_ref()?.rechecks.last()?.wrong_finding);
        experiment.rechecks.entry(trial).or_default().push(Recheck {
            passed,
            reason: reason.clone(),
            wrong_finding,
            recorded_at: recorded_at.clone(),
        });
    }
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
