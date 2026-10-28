//! Whole-Spec review on a Custom process's Spec branch. Ticket accounting
//! supplies the PR and checklist; publication remains in Delivery.

use super::*;

pub(super) fn run(
    prepared: &Prepared,
    step: &ResolvedStep,
    index: usize,
    run: &mut IssueRun<'_>,
) -> Result<String> {
    let tickets = run
        .spec_tickets
        .as_ref()
        .context("spec-review needs land-tickets first")?;
    let text = prompt::spec_review(run.issue, run.base, run.branch, &tickets.pr_url);
    run.pull_request.begin_delivery();
    run.pull_request.ensure_actionable(false)?;
    let checkout = run
        .checkout
        .as_ref()
        .context("the Spec-branch worktree is gone")?;
    checkout.fast_forward_to_origin()?;
    progress::step(format_args!(
        "running step {} with {}",
        step.name, step.choice
    ));
    let logs = session::Logs::of_process_review(&run.issue.repo(), index + 1);
    let before_each = || prepare_for_session(run.issue, prepared.started, checkout.path());
    let (result, log) = session::Sessions::checked_within(
        &logs,
        checkout.path(),
        &step.choice,
        None,
        None,
        &before_each,
        |sessions| {
            let reviewed = sessions.run(session::Kind::SpecReview, &text);
            retain_killed_work(&prepared.killed_work, index + 1, sessions);
            reviewed?;
            run.pull_request.ensure_actionable(false)?;
            Ok(tickets.pr_url.clone())
        },
    );
    if let Err(error) = &result {
        let context = continuation(run);
        let mut failed = preserve_step_failure(
            run.checkout
                .as_mut()
                .expect("the Spec review retains its worktree"),
            &mut run.pull_request,
            context,
            run.on_origin,
            error,
            recovery::Stage::SpecReview,
        );
        failed.log = log;
        run.failure = Some(failed);
    }
    result
}

pub(super) fn validate(path: &Path, file: &ProcessFile) -> Result<()> {
    let spec = file
        .worktrees
        .iter()
        .find(|worktree| worktree.kind == WorktreeKind::SpecBranch);
    for (index, step) in file.steps.iter().enumerate() {
        let DraftKind::SpecReview { line } = step.kind else {
            continue;
        };
        let reason = if !spec.is_some_and(|spec| {
            step.worktree
                .as_ref()
                .is_some_and(|(name, _)| name == &spec.name)
        }) {
            Some("spec-review needs a Spec-branch worktree")
        } else if step.optional || step.parallel.is_some() {
            Some("spec-review must be required and outside a parallel group")
        } else if !file.steps[..index]
            .iter()
            .any(|earlier| matches!(earlier.kind, DraftKind::LandTickets { .. }))
        {
            Some("spec-review needs land-tickets first")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(rule(path, line, "spec-review", reason));
        }
    }
    Ok(())
}
