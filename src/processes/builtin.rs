//! Compiled sequences run inside dispatch's existing Delivery lifecycle.
//! Refuse settings this adapter cannot honor instead of silently ignoring them.

use anyhow::{Result, bail, ensure};

use super::{DraftKind, StepDraft, WorktreeKind, load_inherited_builtin};

pub(crate) enum Step {
    Implement,
    SpecReview,
    SecurityReview,
    Delivery,
    SelfMerge,
}

pub(super) fn steps(name: &str, worktree: WorktreeKind) -> Result<Vec<StepDraft>> {
    let file = load_inherited_builtin(name)?;
    ensure!(
        file.worktrees.len() == 1 && file.worktrees[0].kind == worktree,
        "{name} needs exactly one worktree of its run type"
    );
    Ok(file.steps)
}

pub(super) fn delivery_step(step: StepDraft) -> Result<Step> {
    match step.kind {
        DraftKind::Implement { .. } => Ok(Step::Implement),
        DraftKind::SpecReview { .. } => Ok(Step::SpecReview),
        DraftKind::SecurityReview { .. } => Ok(Step::SecurityReview),
        DraftKind::Delivery { .. } => Ok(Step::Delivery),
        DraftKind::SelfMerge { .. } => Ok(Step::SelfMerge),
        DraftKind::Skill { .. }
        | DraftKind::Command { .. }
        | DraftKind::RunPublishedIssue { .. }
        | DraftKind::LandTickets { .. }
        | DraftKind::ArchitectureReview { .. }
        | DraftKind::SecurityAudit { .. }
        | DraftKind::ReproduceFindings { .. }
        | DraftKind::FixFindings { .. }
        | DraftKind::CloseFindings { .. } => {
            bail!("unsupported Built-in delivery Step {}", step.name)
        }
    }
}
