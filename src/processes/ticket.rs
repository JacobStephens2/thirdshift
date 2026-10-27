//! The compiled Ticket run's sequence, validated with the Custom process rules.
//! Dispatch has already chosen and Claimed its Issue-branch worktree. Its
//! existing Delivery envelope supplies session identity and failure handling.

use anyhow::{Result, bail, ensure};

use super::{Catalog, DraftKind, validated};

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
