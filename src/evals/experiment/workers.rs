//! Two blocking workers take an alternating Arm queue. Each attempt gets a
//! fresh prepared repository; only terminal Trial results are skipped on resume.

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::{Arm, Task, execution, store, trial_results};
use crate::session::limits;
use crate::trial::{Outcome, Output, VoidCause};
use crate::{interrupt, logs, progress};

struct Trial<'a> {
    task: &'a Task,
    arm: Arm,
    index: usize,
}

impl Trial<'_> {
    fn key(&self) -> String {
        self.identity().key()
    }

    fn identity(&self) -> trial_results::Identity<'_> {
        trial_results::Identity {
            suite: &self.task.suite,
            task: &self.task.name,
            arm: self.arm,
            trial: self.index,
        }
    }
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
    let execution = execution::Execution::prepare(
        &experiment.directory,
        prepared,
        [
            &experiment.description.baseline,
            &experiment.description.candidate,
        ],
        Duration::from_secs(experiment.description.bar.timeout_seconds),
        &signal,
    )?;
    let mut queue = VecDeque::new();
    for task in &experiment.tasks {
        for index in 0..experiment.description.bar.trials_per_task[&task.suite] {
            for arm in [Arm::Baseline, Arm::Candidate] {
                let trial = Trial { task, arm, index };
                if !trial_results::finished(&experiment.directory, &trial.identity()) {
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
                    if let Err(error) = run_trial(experiment, &trial, &execution, &signal, &context)
                    {
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
    execution: &execution::Execution<'_>,
    signal: &Path,
    context: &logs::trace::Context,
) -> Result<()> {
    let mut writer = trial_results::Writer::resume(&experiment.directory, trial.identity())?;
    let bar = &experiment.description.bar;
    while writer.progress().outcome == Outcome::Void
        && writer.progress().attempts.len() <= bar.void_rules.retries
    {
        interrupt::check()?;
        limits::check(signal)?;
        let retry = execution.run(
            execution::Request {
                task: trial.task,
                identity: trial.identity(),
                attempt: writer.progress().attempts.len(),
                trace: context,
            },
            |attempt, protocol, score| {
                let outcome = protocol.outcome;
                let void_cause = protocol.void_cause;
                let paused = pauses(&protocol);
                let retry = outcome == Outcome::Void
                    && void_cause.is_some_and(|cause| bar.void_rules.causes.contains(&cause));
                // Preserve completed void attempts even if Stop cuts the next retry
                // short. A resume keeps the original Bar's remaining retry budget.
                writer.record_attempt(&attempt, protocol, score)?;
                if paused {
                    // A provider limit still stops scheduling, including on the last
                    // attempt. Publish exhaustion so resume cannot grant more retries.
                    if writer.progress().attempts.len() > bar.void_rules.retries {
                        writer.finish()?;
                    }
                    bail!(
                        "Trial {} paused: {:?}; run the Experiment again to resume",
                        trial.key(),
                        void_cause.unwrap()
                    );
                }
                Ok(retry)
            },
        )?;
        if !retry {
            break;
        }
    }
    writer.finish()?;
    progress::step(format!(
        "Trial {} {}",
        trial.key(),
        writer.progress().outcome
    ));
    Ok(())
}
