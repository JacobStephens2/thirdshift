//! `evals check-runner`: a Custom runner's static checks and optional dry run.

use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use super::arms::{Check, Runner};
use super::experiment::dry_run::{self, DryRun};

pub struct Request {
    pub runner: String,
    pub task: Option<(String, String)>,
    pub json: bool,
}

#[derive(Serialize)]
pub(crate) struct Report {
    pub version: u32,
    pub runner: String,
    pub manifest: Option<PathBuf>,
    /// The `runner_digest` an Experiment's Setup would record; absent when
    /// the runner could not be frozen.
    pub digest: Option<String>,
    /// Every static check passed and, for a dry run, its Trial's Score passed.
    pub ok: bool,
    pub problems: Vec<Problem>,
    /// Absent without `--suite` and `--task`, or when a static check failed.
    pub trial: Option<DryRun>,
}

#[derive(Serialize)]
pub(crate) struct Problem {
    pub check: Check,
    pub message: String,
}

/// The report, rendered as `--json` or text, and whether it is `ok`.
pub fn execute(request: &Request) -> Result<(String, bool)> {
    let report = check(request)?;
    let text = if request.json {
        serde_json::to_string_pretty(&report)?
    } else {
        render(&report)
    };
    Ok((text, report.ok))
}

fn check(request: &Request) -> Result<Report> {
    // Refuse an unusable Eval task before spending time on the runner.
    let task = request
        .task
        .as_ref()
        .map(|(suite, task)| dry_run::task(suite, task))
        .transpose()?;
    let frozen = tempfile::Builder::new()
        .prefix("thirdshift-check-runner-")
        .tempdir()?;
    let mut problems = Vec::new();
    let runner = Runner::freeze_reporting(&request.runner, frozen.path(), &mut |check, error| {
        problems.push(Problem {
            check,
            message: format!("{error:#}"),
        });
        Ok(())
    })?;
    let digest = match &runner {
        Some(runner) if problems.is_empty() => Some(runner.digest()?),
        _ => None,
    };
    if let Some(runner) = &runner {
        let missing = crate::sandbox::custom::missing_logins(runner, &crate::config::home()?)?;
        for (check, messages) in [
            (Check::Logins, missing.harnesses),
            (Check::Providers, missing.providers),
        ] {
            problems.extend(
                messages
                    .into_iter()
                    .map(|message| Problem { check, message }),
            );
        }
        let prices = crate::trial::accounting::Prices::default();
        for (name, role) in &runner.manifest.roles {
            if !prices.models.contains_key(&role.model) {
                problems.push(Problem {
                    check: Check::Roles,
                    message: format!(
                        "role {name} names Model {}, which the Trial price table ({}) does not price",
                        role.model, prices.date
                    ),
                });
            }
        }
    }
    let trial = match (runner, task) {
        (Some(runner), Some(task)) if problems.is_empty() => Some(dry_run::run(&runner, &task)?),
        _ => None,
    };
    Ok(Report {
        version: 1,
        runner: request.runner.clone(),
        manifest: Runner::manifest_path(&request.runner).ok(),
        digest,
        ok: problems.is_empty() && trial.as_ref().is_none_or(|trial| trial.passed),
        problems,
        trial,
    })
}

fn render(report: &Report) -> String {
    let mut lines = vec![match &report.manifest {
        Some(path) => format!("Custom runner {} ({})", report.runner, path.display()),
        None => format!("Custom runner {}", report.runner),
    }];
    if let Some(digest) = &report.digest {
        lines.push(format!("Digest: {digest}"));
    }
    if report.problems.is_empty() {
        lines.push("Static checks: passed".into());
    } else {
        lines.push("Static checks: failed".into());
        for problem in &report.problems {
            let check = serde_json::to_value(problem.check).unwrap_or_default();
            lines.push(format!(
                "  {}: {}",
                check.as_str().unwrap_or("check"),
                problem.message
            ));
        }
    }
    if let Some(trial) = &report.trial {
        lines.push(format!(
            "Dry run {}/{}: {}{}; Score {}",
            trial.suite,
            trial.task,
            trial.outcome,
            trial
                .void_cause
                .map(|cause| format!(" ({cause})"))
                .unwrap_or_default(),
            if trial.passed { "passed" } else { "failed" }
        ));
        if let Some(cause) = &trial.cause {
            lines.push(format!("  Cause: {cause}"));
        }
        for check in &trial.checks {
            lines.push(format!(
                "  {} {}",
                if check.passed { "passed" } else { "failed" },
                check.reason
            ));
        }
        for session in &trial.sessions {
            let models = session
                .models
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(match (&session.cost_usd, &session.gap) {
                (Some(cost), _) => format!("  Session {}: ${cost:.6} ({models})", session.role),
                (None, Some(gap)) => format!("  Session {}: unpriced: {gap}", session.role),
                (None, None) => format!("  Session {}: unpriced", session.role),
            });
        }
        match trial.cost_usd {
            Some(cost) => lines.push(format!("  Cost: ${cost:.6}")),
            None if !trial.sessions.is_empty() => lines.push("  Cost: incomplete".into()),
            None => lines.push("  Sessions: none recorded".into()),
        }
    }
    lines.push(if report.ok {
        "Custom runner check passed.".into()
    } else {
        "Custom runner check failed.".into()
    });
    lines.join("\n")
}
