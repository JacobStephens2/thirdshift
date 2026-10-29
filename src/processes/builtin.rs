//! Compiled sequences run inside dispatch's existing Delivery lifecycle.
//! Refuse settings this adapter cannot honor instead of silently ignoring them.

use anyhow::{Result, bail, ensure};

use super::{Catalog, DraftKind, StepDraft, WorktreeKind, validated};

pub(crate) enum Step {
    Implement,
    SpecReview,
    SecurityReview,
    Delivery,
    SelfMerge,
}

pub(super) fn steps(name: &str, worktree: WorktreeKind) -> Result<Vec<StepDraft>> {
    let (path, text) = Catalog::installed()?.source(name)?;
    let file = validated(&path, &text)?;
    ensure!(
        file.worktrees.len() == 1 && file.worktrees[0].kind == worktree,
        "{name} needs exactly one worktree of its run type"
    );
    ensure!(
        file.limits.repair_cap == crate::delivery::MAX_REPAIRS
            && !file.limits.forbid_self_merge
            && file.limits.protected_paths.is_empty(),
        "{name} inherits the Command's limits"
    );
    for step in &file.steps {
        ensure!(
            step.harness.is_none()
                && step.model.is_none()
                && step.effort.is_none()
                && !step.optional
                && step.parallel.is_none(),
            "{name} Step {} must inherit the Command's choices and run in sequence",
            step.name
        );
    }
    Ok(file.steps)
}

pub(super) fn delivery_step(step: StepDraft) -> Result<Step> {
    match step.kind {
        DraftKind::Implement { .. } => Ok(Step::Implement),
        DraftKind::SpecReview { .. } => Ok(Step::SpecReview),
        DraftKind::SecurityReview { .. } => Ok(Step::SecurityReview),
        DraftKind::Delivery { .. } => Ok(Step::Delivery),
        DraftKind::SelfMerge { .. } => Ok(Step::SelfMerge),
        _ => bail!("unsupported Built-in delivery Step {}", step.name),
    }
}
