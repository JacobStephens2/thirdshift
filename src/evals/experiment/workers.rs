//! Two blocking workers take an alternating Arm queue. Each attempt gets a
//! fresh prepared repository; only terminal Trial results are skipped on resume.

use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Task, arms, store};
use crate::process::{self, Control, Interruption};
use crate::{interrupt, logs, progress};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Arm {
    Baseline,
    Candidate,
}

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct TrialResult {
    suite: String,
    task: String,
    arm: Arm,
    trial: usize,
    outcome: String,
    pub attempts: Vec<Value>,
}

struct Trial<'a> {
    task: &'a Task,
    arm: Arm,
    index: usize,
}

impl Trial<'_> {
    fn key(&self) -> String {
        format!(
            "{}--{}--{}--{}",
            self.task.suite,
            self.task.name,
            self.arm.name(),
            self.index
        )
    }
}

pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let prepared = experiment
        .description
        .prepared
        .as_ref()
        .context("Arms not prepared")?;
    for (name, arm) in [
        ("baseline", &prepared.baseline),
        ("candidate", &prepared.candidate),
    ] {
        let directory = experiment.directory.join("harnesses").join(name);
        fs::create_dir_all(&directory)?;
        let link = directory.join(&arm.harness.name);
        if !link.exists() {
            symlink(&arm.harness.path, link)?;
        }
    }
    fs::create_dir_all(experiment.directory.join("results"))?;
    let mut queue = VecDeque::new();
    for task in &experiment.tasks {
        for index in 0..experiment.description.bar.trials_per_task[&task.suite] {
            for arm in [Arm::Baseline, Arm::Candidate] {
                let trial = Trial { task, arm, index };
                if !experiment
                    .directory
                    .join("results")
                    .join(format!("{}.json", trial.key()))
                    .exists()
                {
                    queue.push_back(trial);
                }
            }
        }
    }
    logs::stage(logs::Stage::Trials);
    progress::step(format!("Trials: {} remaining, two workers", queue.len()));
    let queue = Mutex::new(queue);
    let stopped = AtomicBool::new(false);
    let results = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..2 {
            workers.push(scope.spawn(|| -> Result<()> {
                loop {
                    let trial = {
                        let mut queue = queue.lock().unwrap();
                        if stopped.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                        interrupt::check()?;
                        queue.pop_front()
                    };
                    let Some(trial) = trial else {
                        return Ok(());
                    };
                    if let Err(error) = run_trial(experiment, &trial, prepared) {
                        stopped.store(true, Ordering::SeqCst);
                        return Err(error);
                    }
                }
            }));
        }
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Trial worker panicked")))
            })
            .collect::<Vec<_>>()
    });
    for result in results {
        result?;
    }
    interrupt::check()
}

fn run_trial(
    experiment: &store::Stored,
    trial: &Trial<'_>,
    prepared: &arms::Prepared,
) -> Result<()> {
    let (arm, setup) = match trial.arm {
        Arm::Baseline => (&prepared.baseline, &experiment.description.baseline),
        Arm::Candidate => (&prepared.candidate, &experiment.description.candidate),
    };
    let directory = experiment.directory.join("trials").join(trial.key());
    fs::create_dir_all(&directory)?;
    let mut result = TrialResult {
        suite: trial.task.suite.clone(),
        task: trial.task.name.clone(),
        arm: trial.arm,
        trial: trial.index,
        outcome: String::new(),
        attempts: Vec::new(),
    };
    let bar = &experiment.description.bar;
    for _ in 0..=bar.void_rules.retries {
        interrupt::check()?;
        let attempt = tempfile::Builder::new()
            .prefix("attempt-")
            .tempdir_in(&directory)?
            .keep();
        let prepared_repo = super::super::repository::prepare(
            &trial.task.directory,
            &trial.task.manifest,
            &attempt.join("repository"),
        )?;
        prepare_branches(&prepared_repo.repository, trial.task)?;
        let inputs = trial
            .task
            .manifest
            .trial
            .as_ref()
            .context("missing Trial inputs")?;
        let request = attempt.join("request.json");
        store::write(
            &request,
            &json!({
                "version": 1, "repository": prepared_repo.repository, "session_kind": "spec-review",
                "inputs": inputs, "harness": setup.harness, "model": setup.model, "effort": setup.effort,
                "output_dir": attempt.join("session")
            }),
        )?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(experiment.directory.join("started"))?;
        progress::step(format!("Trial {} starting", trial.key()));
        let output = execute(
            experiment,
            arm,
            trial.arm,
            &request,
            Duration::from_secs(bar.timeout_seconds),
        );
        interrupt::check()?;
        let protocol = match output {
            Ok(output) => {
                fs::write(attempt.join("stdout"), &output.stdout)?;
                fs::write(attempt.join("stderr"), &output.stderr)?;
                if !output.status.success() {
                    bail!(
                        "Arm Trial protocol failed ({}): {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                let protocol: Value =
                    serde_json::from_slice(&output.stdout).context("invalid Trial result")?;
                ensure!(
                    protocol["version"] == 1
                        && matches!(
                            protocol["outcome"].as_str(),
                            Some("succeeded" | "failed" | "void")
                        ),
                    "unsupported Trial result"
                );
                protocol
            }
            Err(error) if error.is::<process::TimedOut>() => {
                json!({ "version": 1, "outcome": "failed", "cause": "Trial timed out", "void_cause": null, "timings": {"total_ms": bar.timeout_seconds * 1000}, "log": null })
            }
            Err(error) => return Err(error),
        };
        store::write(&attempt.join("result.json"), &protocol)?;
        let outcome = protocol["outcome"].as_str().unwrap();
        let void_cause = protocol["void_cause"].as_str();
        if outcome == "void"
            && matches!(
                void_cause,
                Some("usage_limit" | "rate_limit" | "interrupted")
            )
        {
            bail!(
                "Trial {} paused: {}; run the Experiment again to resume",
                trial.key(),
                void_cause.unwrap()
            );
        }
        result.outcome = outcome.into();
        let retry = outcome == "void"
            && void_cause
                .is_some_and(|cause| bar.void_rules.causes.iter().any(|rule| rule == cause));
        result.attempts.push(protocol);
        if !retry {
            break;
        }
    }
    store::write(
        &experiment
            .directory
            .join("results")
            .join(format!("{}.json", trial.key())),
        &result,
    )?;
    progress::step(format!("Trial {} {}", trial.key(), result.outcome));
    Ok(())
}

fn prepare_branches(repository: &Path, task: &Task) -> Result<()> {
    let inputs = task
        .manifest
        .trial
        .as_ref()
        .context("missing Trial inputs")?;
    let git = crate::git::Git::new(repository);
    git.run(&["check-ref-format", "--branch", &inputs.base])?;
    git.run(&["check-ref-format", "--branch", &inputs.branch])?;
    ensure!(
        inputs.base != inputs.branch,
        "Trial Base branch and Issue branch must differ"
    );
    git.run(&["branch", &inputs.base, &task.manifest.base])?;
    git.run(&["checkout", "-b", &inputs.branch])?;
    git.run(&[
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "user.name=Eval",
        "-c",
        "user.email=eval@example.invalid",
        "commit",
        "--allow-empty",
        "-m",
        "Eval change",
    ])?;
    git.run(&["push", "origin", &inputs.base, &inputs.branch])?;
    Ok(())
}

fn execute(
    experiment: &store::Stored,
    arm: &arms::Arm,
    name: Arm,
    request: &Path,
    timeout: Duration,
) -> Result<std::process::Output> {
    let directory = experiment.directory.join("harnesses").join(name.name());
    let path = std::env::join_paths(std::iter::once(directory).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))?;
    process::output_until(
        Command::new(&arm.binary)
            .arg("--trial")
            .arg(request)
            .env("PATH", path),
        None,
        Control {
            name: "Trial",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        Instant::now() + timeout,
    )
}
