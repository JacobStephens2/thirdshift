//! Built-in and Custom Ticket execution, validated with the Custom process rules.
//! Dispatch has already chosen its Issue-branch worktree and made any Claim. Its
//! existing Delivery envelope supplies session identity and failure handling.

use anyhow::Result;

use super::{WorktreeKind, builtin};

/// Run a Custom Ticket Process in the worktree dispatch already acquired.
/// The owning Command, Claim, Base branch and child outcome stay with dispatch.
pub(crate) fn run_custom(
    name: &str,
    delivery: crate::delivery::Delivery<'_>,
    worktree: crate::worktree::Worktree,
    selection: &crate::branch::Selection,
    spec_pr: Option<crate::child_run::SpecPr>,
) -> Result<crate::run::Reached, crate::failed_run::FailedRun> {
    use super::*;

    let (path, mut file, identity) = load(name)?;
    logs::process_running(identity.clone());
    let config = UserConfig::load()?;
    file.goal = delivery.goal;
    file.security_review = delivery.security_review;
    file.security_fix = delivery.child_asks.security_fix;
    file.security_flags.security_fix = Some(if file.security_fix {
        crate::security::FixAsk::Allow
    } else {
        crate::security::FixAsk::Forbid
    });
    file.security_flags.security_close = Some(if delivery.child_asks.security_close {
        crate::security::CloseAsk::Allow
    } else {
        crate::security::CloseAsk::Forbid
    });
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
    // The parent Command owns the shared Factory-skill folder until all
    // its Steps, including parallel siblings and later sessions, finish.
    prepared._written.remove_on_drop = false;
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
        pull_request: {
            let mut pull_request =
                PullRequest::new(delivery.issue, &branch, delivery.base, GitHub::new());
            if let Some(spec_pr) = spec_pr {
                pull_request.gate_self_merge_on(spec_pr);
            }
            pull_request
        },
        reviews: Vec::new(),
        spec_tickets: None,
        child_asks: delivery.child_asks.clone(),
        failure: None,
    };
    let result = execute_issue_steps(name, &prepared, &mut issue_run);
    if let Err(error) = result {
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
    let resolution = if let Some(reason) = completed.architecture_end.filter(|_| own_pr.is_none()) {
        crate::run::Resolution::ArchitectureReview {
            url: result.expect("the Process completed successfully"),
            reason,
        }
    } else if let Some(comment_url) = completed.unreproduced_bug {
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

pub(crate) fn steps() -> Result<Vec<builtin::Step>> {
    builtin::steps("ticket-run", WorktreeKind::IssueBranch)?
        .into_iter()
        .map(builtin::delivery_step)
        .collect()
}
