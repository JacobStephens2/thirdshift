//! Two blocking workers take an alternating Arm queue. Each attempt gets a
//! fresh prepared repository; only terminal Trial results are skipped on resume.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{Task, arms, grading, integrity, store};
use crate::process::{self, Control, Interruption};
use crate::session::limits;
use crate::trial::{Outcome, Output, VoidCause};
use crate::{interrupt, logs, progress};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Arm {
    Baseline,
    Candidate,
}

impl Arm {
    pub fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }

    pub fn from_trial_key(key: &str) -> Option<Self> {
        let name = key.split("--").nth(1)?;
        [Self::Baseline, Self::Candidate]
            .into_iter()
            .find(|arm| arm.name() == name)
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct TrialResult {
    pub suite: String,
    pub task: String,
    pub arm: Arm,
    pub trial: usize,
    pub outcome: Outcome,
    #[serde(default)]
    pub score: Option<grading::Score>,
    pub attempts: Vec<Output>,
}

impl TrialResult {
    pub fn key(&self) -> String {
        trial_key(&self.suite, &self.task, self.arm, self.trial)
    }
}

struct Trial<'a> {
    task: &'a Task,
    arm: Arm,
    index: usize,
}

impl Trial<'_> {
    fn key(&self) -> String {
        trial_key(&self.task.suite, &self.task.name, self.arm, self.index)
    }
}

fn trial_key(suite: &str, name: &str, arm: Arm, index: usize) -> String {
    // Names cannot contain NUL. Hash the delimited tuple so names with
    // `--` stay distinct and long names cannot exceed a filename limit.
    let task = Sha256::new()
        .chain_update(suite.as_bytes())
        .chain_update([0])
        .chain_update(name.as_bytes())
        .finalize();
    format!("{task:x}--{}--{}", arm.name(), index)
}

fn pauses(output: &Output) -> bool {
    output.outcome == Outcome::Void
        && matches!(
            output.void_cause,
            Some(VoidCause::UsageLimit | VoidCause::RateLimit | VoidCause::Interrupted)
        )
}

pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let signal = experiment.directory.join("plan-limit");
    match fs::remove_file(&signal) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
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
    let context = logs::trace_context().context("Experiment has no Trace context")?;
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
                        limits::check(&signal)?;
                        queue.pop_front()
                    };
                    let Some(trial) = trial else {
                        return Ok(());
                    };
                    if let Err(error) = run_trial(experiment, &trial, prepared, &signal, &context) {
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
    signal: &Path,
    context: &logs::trace::Context,
) -> Result<()> {
    let (arm, setup) = match trial.arm {
        Arm::Baseline => (&prepared.baseline, &experiment.description.baseline),
        Arm::Candidate => (&prepared.candidate, &experiment.description.candidate),
    };
    let directory = experiment.directory.join("trials").join(trial.key());
    fs::create_dir_all(&directory)?;
    let pending = directory.join("pending.json");
    let terminal = experiment
        .directory
        .join("results")
        .join(format!("{}.json", trial.key()));
    let mut result = if pending.exists() {
        serde_json::from_slice(&fs::read(&pending)?)?
    } else {
        TrialResult {
            suite: trial.task.suite.clone(),
            task: trial.task.name.clone(),
            arm: trial.arm,
            trial: trial.index,
            outcome: Outcome::Void,
            score: None,
            attempts: Vec::new(),
        }
    };
    let bar = &experiment.description.bar;
    while result.outcome == Outcome::Void && result.attempts.len() <= bar.void_rules.retries {
        interrupt::check()?;
        limits::check(signal)?;
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
        crate::skills::exclude(
            &crate::git::Git::new(&prepared_repo.repository),
            &format!("/{}", crate::prompt::ISSUE_FILE),
        )?;
        // Establish the Harness's idempotent instruction fallback before the
        // comparison. The session may read it, but editing it still voids a Trial.
        setup
            .choice()
            .harness
            .adapter()
            .link_instruction_fallback(&prepared_repo.repository)?;
        let before = integrity::snapshot(&prepared_repo.repository)?;
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
                "version": 1, "repository": prepared_repo.repository, "session_kind": trial.task.manifest.session,
                "inputs": inputs, "harness": setup.harness, "model": setup.model, "effort": setup.effort,
                "output_dir": attempt.join("session")
            }),
        )?;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(experiment.directory.join("started"))?;
        limits::check(signal)?;
        progress::step(format!("Trial {} starting", trial.key()));
        let monitor = RefCell::new(super::limits::Monitor::new(
            &attempt.join("session"),
            &prepared_repo.repository,
            &setup.choice(),
        ));
        let started = Instant::now();
        let mut trace = super::trace::Attempt::start(&attempt, &trial.key(), context)?;
        let mut command = trial_command(experiment, arm, trial.arm, &request, signal)?;
        trace.inherit(&mut command)?;
        let output = process::output_until(
            &mut command,
            None,
            Control {
                name: "Trial",
                interruption: Interruption::Ordinary,
                stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
            },
            Instant::now() + Duration::from_secs(bar.timeout_seconds),
            &|| monitor.borrow_mut().check(signal),
        );
        interrupt::check()?;
        let mut protocol = match output {
            Ok(output) => {
                fs::write(attempt.join("stdout"), &output.stdout)?;
                fs::write(attempt.join("stderr"), &output.stderr)?;
                if output.status.signal() == Some(libc::SIGKILL) {
                    Output::failed(
                        started.elapsed().as_millis(),
                        "Trial process killed (SIGKILL); external cause unknown",
                    )
                } else {
                    if !output.status.success() {
                        bail!(
                            "Arm Trial protocol failed ({}): {}",
                            output.status,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    let protocol: Output =
                        serde_json::from_slice(&output.stdout).context("invalid Trial result")?;
                    ensure!(protocol.version == 1, "unsupported Trial result");
                    protocol
                }
            }
            Err(error) if error.is::<process::TimedOut>() => {
                Output::failed(u128::from(bar.timeout_seconds) * 1000, "Trial timed out")
            }
            Err(error) if error.is::<limits::Reached>() => monitor
                .into_inner()
                .cut_short(started.elapsed().as_millis()),
            Err(error) => return Err(error),
        };
        let after = integrity::snapshot(&prepared_repo.repository);
        interrupt::check()?;
        // Keep known failures and external Void causes, including limit
        // stops. Repository changes invalidate an otherwise successful Trial.
        if protocol.outcome == Outcome::Succeeded
            && !after.as_ref().is_ok_and(|after| after == &before)
        {
            protocol.outcome = Outcome::Void;
            protocol.void_cause = Some(VoidCause::RepositoryChanged);
            protocol.cause = Some("Trial repository changed during its session".into());
        }
        let score = trace.grade(|| {
            match grading::score(trial.task, &prepared_repo.repository, &protocol, bar) {
                Err(error) if error.is::<crate::sandbox::Unavailable>() => {
                    protocol.outcome = Outcome::Void;
                    protocol.void_cause = Some(VoidCause::SandboxUnavailable);
                    protocol.cause = Some(format!("{error:#}"));
                    grading::score(trial.task, &prepared_repo.repository, &protocol, bar)
                }
                score => score,
            }
        })?;
        store::write(&attempt.join("result.json"), &protocol)?;
        let outcome = protocol.outcome;
        let void_cause = protocol.void_cause;
        let paused = pauses(&protocol);
        result.outcome = outcome;
        result.score = Some(score);
        trace.outcome(outcome);
        let retry = outcome == Outcome::Void
            && void_cause.is_some_and(|cause| bar.void_rules.causes.contains(&cause));
        result.attempts.push(protocol);
        // Preserve completed void attempts even if Stop cuts the next retry
        // short. A resume keeps the original Bar's remaining retry budget.
        store::write(&pending, &result)?;
        if paused {
            // A provider limit still stops scheduling, including on the last
            // attempt. Publish exhaustion so resume cannot grant more retries.
            if result.attempts.len() > bar.void_rules.retries {
                store::write(&terminal, &result)?;
            }
            bail!(
                "Trial {} paused: {:?}; run the Experiment again to resume",
                trial.key(),
                void_cause.unwrap()
            );
        }
        if !retry {
            break;
        }
    }
    store::write(&terminal, &result)?;
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
    if task.manifest.session == super::super::format::Session::SpecReview {
        git.run(&["push", "origin", &inputs.base, &inputs.branch])?;
    }
    Ok(())
}

fn trial_command(
    experiment: &store::Stored,
    arm: &arms::Arm,
    name: Arm,
    request: &Path,
    signal: &Path,
) -> Result<Command> {
    let directory = experiment.directory.join("harnesses").join(name.name());
    let path = std::env::join_paths(std::iter::once(directory).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))?;
    let mut command = Command::new(&arm.binary);
    command
        .arg("--trial")
        .arg(request)
        .env(limits::SIGNAL_ENV, signal)
        .env("PATH", path);
    Ok(command)
}
