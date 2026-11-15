//! One throwaway Custom-runner Trial, outside Experiment storage.
//!
//! The Trial runs through the same execution as an Experiment's: the real
//! sandbox, Session evidence, pricing, Void checks and code checks. Nothing
//! is retained: its directory is removed when the dry run ends.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::{Arm, Bar, Setup, Task, arms, execution, format, suite, trial_results};
use crate::sandbox::code_grader::Check;
use crate::session::usage::Measurement;
use crate::trial::{Outcome, VoidCause};

#[derive(Serialize)]
pub(crate) struct DryRun {
    pub suite: String,
    pub task: String,
    pub outcome: Outcome,
    pub void_cause: Option<VoidCause>,
    pub cause: Option<String>,
    /// The Trial's Score: its outcome check and the task's code checks.
    pub passed: bool,
    pub checks: Vec<Check>,
    pub sessions: Vec<Session>,
    /// Absent when any Session could not be priced.
    pub cost_usd: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct Session {
    pub role: String,
    pub time_ms: u128,
    pub models: BTreeMap<String, Measurement>,
    pub cost_usd: Option<f64>,
    pub gap: Option<String>,
}

/// The process Eval task a dry run would use, refused before any check runs.
pub(crate) fn task(suite: &str, task: &str) -> Result<TaskInput> {
    super::runner_suite(suite)?;
    let root = crate::config::home()?.join(".thirdshift/evals/suites");
    let directory = root.join(suite);
    suite::Manifest::load(&directory)?;
    let directory = directory.join("tasks").join(task);
    let manifest = format::Task::load(&directory)
        .with_context(|| format!("cannot load Eval task {suite}/{task}"))?;
    ensure!(
        manifest.session == format::Session::Process,
        "Custom runners require process Eval tasks: {suite}/{task}"
    );
    super::trial_inputs(
        manifest
            .trial
            .as_ref()
            .with_context(|| format!("Eval task {suite}/{task} needs a [trial] table"))?,
    )?;
    Ok(TaskInput(Task {
        suite: suite.into(),
        name: task.into(),
        directory,
        manifest,
    }))
}

pub(crate) struct TaskInput(Task);

/// Run one Candidate Trial of `runner` on `task` in a private directory.
pub(crate) fn run(runner: &arms::Runner, task: &TaskInput) -> Result<DryRun> {
    let task = &task.0;
    crate::interrupt::install()?;
    let directory = tempfile::Builder::new()
        .prefix("thirdshift-dry-run-")
        .tempdir()?;
    let root = directory.path().canonicalize()?;
    let prepared = arms::Prepared::runners(runner.clone());
    let setup = Setup {
        commit: None,
        harness: None,
        model: None,
        effort: None,
        process: None,
        process_digest: None,
        runner: Some(runner.name.clone()),
        runner_digest: Some(runner.digest()?),
    };
    let signal = root.join("plan-limit");
    let prices = crate::trial::accounting::Prices::default();
    let execution = execution::Execution::prepare(
        &root,
        &prepared,
        [&setup, &setup],
        Duration::from_secs(Bar::default().timeout_seconds),
        &signal,
    )?
    .with_pricing(Some(&prices));
    let trace = crate::logs::trace::Context::of_session(&root);
    execution.run(
        execution::Request {
            task,
            identity: trial_results::Identity {
                suite: &task.suite,
                task: &task.name,
                arm: Arm::Candidate,
                trial: 0,
            },
            attempt: 0,
            trace: &trace,
        },
        |_, output, score| {
            let sessions: Vec<Session> = output
                .accounting
                .map(|accounting| accounting.sessions)
                .unwrap_or_default()
                .into_iter()
                .map(|session| Session {
                    role: session.role,
                    time_ms: session.time_ms,
                    models: session.models,
                    cost_usd: session.cost_usd,
                    gap: session.gap,
                })
                .collect();
            let cost_usd = sessions
                .iter()
                .map(|session| session.cost_usd)
                .sum::<Option<f64>>();
            Ok(DryRun {
                suite: task.suite.clone(),
                task: task.name.clone(),
                outcome: output.outcome,
                void_cause: output.void_cause,
                cause: output.cause,
                passed: score.passed,
                checks: score.checks,
                sessions,
                cost_usd,
            })
        },
    )
}
