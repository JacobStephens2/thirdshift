//! Human decisions supplement immutable Grader Scores. One locked metadata
//! write publishes their history, recalculated Verdict and pending list together.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::grading::task_is_void;
use super::store::{State, Stored};

pub(super) type History = BTreeMap<String, Vec<Recheck>>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Recheck {
    pub passed: bool,
    pub reason: String,
    pub wrong_finding: Option<bool>,
    pub recorded_at: String,
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
