//! The compiled Architect Process, through the Pass's existing adapters so its
//! session identity, child asks, progress and endings stay with the Command.

use anyhow::{Context, Result, anyhow, bail, ensure};
use chrono::{DateTime, Utc};

use super::{Catalog, DraftKind, validated};
use crate::architect::{Outcome, Reviewed, conclude};
use crate::failed_run::FailedRun;
use crate::issue::Repo;
use crate::logs::Work;
use crate::pass::{Dispatch, Outside};
use crate::prompt;

#[cfg(test)]
mod tests;

/// The Command's already resolved choices, after its lock and skip gates.
pub(crate) struct ArchitectRun<'a> {
    pub repo: &'a Repo,
    pub base: &'a str,
    pub origin: &'a str,
    pub focus: Option<&'a str>,
    pub started: DateTime<Utc>,
    pub pull: bool,
    pub plan_only: bool,
}

pub(crate) fn run(outside: &mut impl Outside, run: &ArchitectRun<'_>) -> Outcome {
    match run_steps(outside, run) {
        Ok(outcome) => outcome,
        Err(failed) => Outcome::Ran {
            review: Err(failed),
            dispatched: None,
        },
    }
}

fn run_steps(outside: &mut impl Outside, run: &ArchitectRun<'_>) -> Result<Outcome, FailedRun> {
    let steps = steps()?;
    outside.check_harness(!run.plan_only)?;
    outside.started(Work::ArchitectRun(run.repo));
    if run.pull {
        outside.pull();
    }
    if outside.interrupted() {
        return Err(FailedRun {
            interrupted: true,
            ..FailedRun::from(anyhow!("interrupted"))
        });
    }
    let mut reviewed = None;
    let mut dispatched = None;
    for step in steps {
        match step {
            DraftKind::ArchitectureReview { .. } => {
                let review = review(outside, run)?;
                let end = run.plan_only || !matches!(review, Reviewed::PlanReady(..));
                reviewed = Some(review);
                if end {
                    break;
                }
            }
            DraftKind::RunPublishedIssue { .. } => {
                let Some(Reviewed::PlanReady(plan, _)) = &reviewed else {
                    return Err(anyhow!(
                        "architect-run needs an Architecture review before dispatch"
                    )
                    .into());
                };
                outside.step(format!(
                    "dispatching the plan {url}, as thirdshift {url} would",
                    url = plan.url
                ));
                dispatched = Some(outside.dispatch(Dispatch::ArchitectPlan {
                    plan,
                    base: run.base,
                }));
            }
            _ => unreachable!("the compiled Architect Steps were checked"),
        }
    }
    Ok(Outcome::Ran {
        review: Ok(reviewed.context("architect-run ended without an Architecture review")?),
        dispatched,
    })
}

fn review(outside: &mut impl Outside, run: &ArchitectRun<'_>) -> Result<Reviewed, FailedRun> {
    let starting = match run.focus {
        Some(focus) => format!(
            "starting the Architecture review of {}, focused on: {focus}",
            run.base
        ),
        None => format!("starting the Architecture review of {}", run.base),
    };
    let prompt = prompt::architecture_review(run.base, run.focus);
    let (reviewed, log) = outside.review(run.base, starting, &prompt, |outside, final_message| {
        conclude(outside, final_message, run.origin, run.started)
    });
    reviewed.map_err(|error| FailedRun {
        log,
        ..FailedRun::from(error)
    })
}

fn steps() -> Result<Vec<DraftKind>> {
    let (path, text) = Catalog::installed()?.source("architect-run")?;
    let file = validated(&path, &text)?;
    // The adapter retains the Pass's checked choices and specialized review
    // worktree. Refuse settings it cannot carry out rather than ignore them.
    ensure!(
        file.worktrees.is_empty(),
        "architect-run owns its review worktree"
    );
    ensure!(
        file.limits.repair_cap == crate::delivery::MAX_REPAIRS
            && !file.limits.forbid_self_merge
            && file.limits.protected_paths.is_empty(),
        "architect-run inherits the Command's limits"
    );
    file.steps
        .into_iter()
        .map(|step| {
            ensure!(
                step.harness.is_none()
                    && step.model.is_none()
                    && step.effort.is_none()
                    && !step.optional
                    && step.parallel.is_none()
                    && step.worktree.is_none()
                    && step.process.is_none(),
                "architect-run Step {} must inherit the Command's choices and run in sequence",
                step.name
            );
            match step.kind {
                DraftKind::ArchitectureReview { .. } | DraftKind::RunPublishedIssue { .. } => {
                    Ok(step.kind)
                }
                _ => bail!("unsupported architect-run Step {}", step.name),
            }
        })
        .collect()
}
