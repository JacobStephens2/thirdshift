//! A complete logical Trial: saved-Bar continuation and retained publication.
//!
//! Only scoped attempt execution varies. Workers never coordinate checkpoints.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::{Arm, Bar, Task, TrialResult, execution, grading, store, trial_results};
use crate::session::limits;
use crate::trial::{Outcome, Output, VoidCause};
use crate::{interrupt, logs, progress};

type Completion<'a> = Box<dyn FnOnce(PathBuf, Output, grading::Score) -> Result<()> + 'a>;

/// Private scoped seam: completion runs while execution evidence is still live.
trait Attempt: Sync {
    fn run(&self, request: execution::Request<'_>, complete: Completion<'_>) -> Result<()>;
}

impl Attempt for execution::Execution<'_> {
    fn run(&self, request: execution::Request<'_>, complete: Completion<'_>) -> Result<()> {
        self.run(request, complete)
    }
}

pub(super) struct Runner<'a> {
    directory: &'a Path,
    bar: &'a Bar,
    signal: &'a Path,
    trace: &'a logs::trace::Context,
    execution: Box<dyn Attempt + 'a>,
}

impl<'a> Runner<'a> {
    /// Shared execution setup is prepared once, before either worker starts.
    pub fn prepare(
        experiment: &'a store::Stored,
        signal: &'a Path,
        trace: &'a logs::trace::Context,
    ) -> Result<Self> {
        let description = &experiment.description;
        let execution = execution::Execution::prepare(
            &experiment.directory,
            description.prepared.as_ref().context("Arms not prepared")?,
            [&description.baseline, &description.candidate],
            Duration::from_secs(description.bar.timeout_seconds),
            signal,
        )?;
        Ok(Self {
            directory: &experiment.directory,
            bar: &description.bar,
            signal,
            trace,
            execution: Box::new(execution),
        })
    }

    /// Resume and complete one Trial, or checkpoint before returning an error/pause.
    pub fn run(&self, task: &Task, arm: Arm, index: usize) -> Result<()> {
        let identity = trial_results::Identity {
            suite: &task.suite,
            task: &task.name,
            arm,
            trial: index,
        };
        let mut writer = Writer::resume(self.directory, &identity)?;
        // Recovery may only need to publish retained history. Never replay its pause.
        if !writer.can_continue(self.bar) {
            writer.finish()?;
        } else {
            loop {
                interrupt::check()?;
                limits::check(self.signal)?;
                self.execution.run(
                    execution::Request {
                        task,
                        identity: trial_results::Identity { ..identity },
                        attempt: writer.progress.attempts.len(),
                        trace: self.trace,
                    },
                    Box::new(|attempt, output, score| {
                        let pause = pause_cause(&output);
                        writer.record_attempt(&attempt, output, score)?;
                        // Terminal publication belongs to the same evidence lease.
                        if !writer.can_continue(self.bar) {
                            writer.finish()?;
                        }
                        if let Some(cause) = pause {
                            limits::check(self.signal)?;
                            bail!(
                                "Trial {} paused: {:?}; run the Experiment again to resume",
                                identity.key(),
                                cause
                            );
                        }
                        Ok(())
                    }),
                )?;
                if !writer.can_continue(self.bar) {
                    break;
                }
            }
        }
        progress::step(format!(
            "Trial {} {}",
            identity.key(),
            writer.progress.outcome
        ));
        Ok(())
    }
}

fn pause_cause(output: &Output) -> Option<VoidCause> {
    if output.outcome == Outcome::Void {
        output.void_cause.filter(|cause| {
            matches!(
                cause,
                VoidCause::UsageLimit | VoidCause::RateLimit | VoidCause::Interrupted
            )
        })
    } else {
        None
    }
}

/// Publication and history are private to the complete Trial operation.
struct Writer {
    pending: PathBuf,
    terminal: PathBuf,
    progress: TrialResult,
}

impl Writer {
    fn resume(directory: &Path, identity: &trial_results::Identity<'_>) -> Result<Self> {
        let key = identity.key();
        let trial = directory.join("trials").join(&key);
        fs::create_dir_all(&trial)?;
        fs::create_dir_all(directory.join("results"))?;
        let pending = trial.join("pending.json");
        let progress = if pending.exists() {
            serde_json::from_slice(&fs::read(&pending)?)?
        } else {
            TrialResult {
                suite: identity.suite.into(),
                task: identity.task.into(),
                arm: identity.arm,
                trial: identity.trial,
                outcome: Outcome::Void,
                score: None,
                attempts: Vec::new(),
            }
        };
        Ok(Self {
            pending,
            terminal: directory.join("results").join(format!("{key}.json")),
            progress,
        })
    }

    fn can_continue(&self, bar: &Bar) -> bool {
        self.progress.attempts.last().is_none_or(|latest| {
            latest.outcome == Outcome::Void
                && latest
                    .void_cause
                    .is_some_and(|cause| bar.void_rules.causes.contains(&cause))
                && self.progress.attempts.len() <= bar.void_rules.retries
        })
    }

    fn record_attempt(
        &mut self,
        attempt: &Path,
        output: Output,
        score: grading::Score,
    ) -> Result<()> {
        store::write(&attempt.join("result.json"), &output)?;
        self.progress.outcome = output.outcome;
        self.progress.score = Some(score);
        self.progress.attempts.push(output);
        // An error returns immediately; only the durable pending prefix can resume.
        store::write(&self.pending, &self.progress)
    }

    fn finish(&self) -> Result<()> {
        store::write(&self.terminal, &self.progress)
    }
}

#[cfg(test)]
mod tests;
