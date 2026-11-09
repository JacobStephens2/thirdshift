//! Two blocking workers take an alternating Arm queue. Each attempt gets a
//! fresh prepared repository; only terminal Trial results are skipped on resume.

use std::collections::VecDeque;
use std::fs;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Context, Result};

use super::{Arm, Task, store, trial, trial_results};
use crate::session::limits;
use crate::{interrupt, logs, progress};

pub(super) struct Trial<'a> {
    pub task: &'a Task,
    pub arm: Arm,
    index: usize,
}

impl Trial<'_> {
    fn identity(&self) -> trial_results::Identity<'_> {
        trial_results::Identity {
            suite: &self.task.suite,
            task: &self.task.name,
            arm: self.arm,
            trial: self.index,
        }
    }
}

/// The Trials without a terminal result, in the order workers take them.
pub(super) fn remaining(experiment: &store::Stored) -> VecDeque<Trial<'_>> {
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
    queue
}

pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let signal = experiment.directory.join("plan-limit");
    match fs::remove_file(&signal) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    logs::stage(logs::Stage::Trials);
    let context = logs::trace_context().context("Experiment has no Trace context")?;
    let runner = trial::Runner::prepare(experiment, &signal, &context)?;
    let queue = remaining(experiment);
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
                    if let Err(error) = runner.run(trial.task, trial.arm, trial.index) {
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
