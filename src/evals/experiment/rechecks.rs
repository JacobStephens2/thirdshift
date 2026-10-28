//! Human decisions supplement immutable Grader Scores. One locked metadata
//! write publishes their history, recalculated Verdict and pending list together.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::grading::{Pending, task_is_void};
use super::store::{State, Stored};

pub(super) type History = BTreeMap<String, Vec<Recheck>>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Recheck {
    pub passed: bool,
    pub reason: String,
    pub wrong_finding: Option<bool>,
    pub recorded_at: String,
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
    super::retention::repositories(&experiment);
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
