//! Built-in and Custom Ticket execution, validated with the Custom process rules.
//! Dispatch has already chosen its Issue-branch worktree and made any Claim. Its
//! existing Delivery envelope supplies session identity and failure handling.

use anyhow::{Result, bail, ensure};

use super::{Catalog, DraftKind, validated};

/// Run a Custom Ticket Process in the worktree dispatch already acquired.
/// The owning Command, Claim, Base branch and child outcome stay with dispatch.
pub(crate) fn run_custom(
    name: &str,
    delivery: crate::delivery::Delivery<'_>,
    worktree: crate::worktree::Worktree,
    selection: &crate::branch::Selection,
) -> Result<crate::run::Reached, crate::failed_run::FailedRun> {
    use super::*;

    let (path, mut file, identity) = load(name)?;
    logs::process_running(identity.clone());
    let config = UserConfig::load()?;
    file.goal = delivery.goal;
    file.security_review = delivery.security_review;
    file.limits.repair_cap = file.limits.repair_cap.min(delivery.repair_cap);
    file.limits
        .protected_paths
        .extend_from_slice(delivery.protected_paths);
    let choice = &delivery.child_asks.harness;
    let asked = Asked {
        harness: Some(choice.harness),
        model_and_effort: harness::ModelAndEffort {
            model: choice.model.clone(),
            effort: choice.effort.clone(),
        },
    };
    let commit = worktree
        .launch()
        .run(&["rev-parse", &format!("origin/{}", delivery.base)])?;
    let mut prepared = prepare_from(
        &path,
        &config,
        &asked,
        file,
        Some(delivery.issue.clone()),
        delivery.started,
        WorktreeSource {
            git: worktree.launch(),
            commit: &commit,
            issue_checkout: Some(worktree.path()),
            base: delivery.base,
        },
    )?;
    prepared.dispatched = Some(identity);
    refuse_unstartable(&prepared)?;
    let branch = worktree.branch().to_string();
    let mut held = Some(worktree);
    let mut issue_run = IssueRun {
        checkout: &mut held,
        issue: delivery.issue,
        branch: &branch,
        base: delivery.base,
        on_origin: matches!(selection, Selection::Continuation { .. }),
        selection,
        logs: delivery.logs,
        base_fix: delivery.base_fix,
        process: name,
        delivered: false,
        pull_request: PullRequest::new(delivery.issue, &branch, delivery.base, GitHub::new()),
        reviews: Vec::new(),
        child_asks: delivery.child_asks.clone(),
        failure: None,
    };
    if let Err(error) = execute_issue_steps(name, &prepared, &mut issue_run) {
        let mut failed = match issue_run.failure.take() {
            Some(mut failed) => {
                failed.error = error;
                failed
            }
            None => crate::failed_run::FailedRun::from(error),
        };
        if failed.pr_url.is_none() {
            failed.pr_url = issue_run.pull_request.info().map(|pr| pr.url.clone());
        }
        return Err(failed);
    }
    let own_pr = issue_run.pull_request.info().map(|pr| pr.url.clone());
    let completed = std::mem::take(&mut *prepared.completed.lock().expect("completed Runs"));
    let resolution = if let Some(comment_url) = completed.unreproduced_bug {
        crate::run::Resolution::NotReproduced { comment_url }
    } else {
        completed
            .resolutions
            .into_iter()
            .find(|resolution| Some(resolution.url()) == own_pr.as_deref())
            .context("the Ticket Process ended without delivery")?
    };
    Ok(crate::run::Reached {
        resolution,
        log: None,
        ticket_lines: Vec::new(),
    })
}

#[cfg(test)]
mod tests;

pub(crate) enum Step {
    Implement,
    SecurityReview,
    Delivery,
    SelfMerge,
}

pub(crate) fn steps() -> Result<Vec<Step>> {
    let (path, text) = Catalog::installed()?.source("ticket-run")?;
    let file = validated(&path, &text)?;
    // This adapter uses the Command's already checked choices and held
    // worktree. Refuse settings it cannot carry out rather than ignore them.
    ensure!(
        file.worktrees.len() == 1 && file.worktrees[0].issue_branch_line.is_some(),
        "ticket-run needs exactly one Issue-branch worktree"
    );
    ensure!(
        file.limits.repair_cap == crate::delivery::MAX_REPAIRS
            && !file.limits.forbid_self_merge
            && file.limits.protected_paths.is_empty(),
        "ticket-run inherits the Command's limits"
    );
    file.steps
        .into_iter()
        .map(|step| {
            ensure!(
                step.harness.is_none()
                    && step.model.is_none()
                    && step.effort.is_none()
                    && !step.optional
                    && step.parallel.is_none(),
                "ticket-run Step {} must inherit the Command's choices and run in sequence",
                step.name
            );
            match step.kind {
                DraftKind::Implement { .. } => Ok(Step::Implement),
                DraftKind::SecurityReview { .. } => Ok(Step::SecurityReview),
                DraftKind::Delivery { .. } => Ok(Step::Delivery),
                DraftKind::SelfMerge { .. } => Ok(Step::SelfMerge),
                _ => bail!("unsupported ticket-run Step {}", step.name),
            }
        })
        .collect()
}
