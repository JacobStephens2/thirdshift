//! The Fix run's compiled sequence enters the shared Delivery directly.

use anyhow::{Result, ensure};

use super::{DraftKind, WorktreeKind, builtin};

pub(crate) fn steps() -> Result<Vec<builtin::Step>> {
    let steps = builtin::steps("fix-run", WorktreeKind::PullRequest)?;
    ensure!(
        steps.len() == 2
            && matches!(steps[0].kind, DraftKind::Delivery { .. })
            && matches!(steps[1].kind, DraftKind::SelfMerge { .. })
            && steps.iter().all(|step| step
                .worktree
                .as_ref()
                .is_some_and(|(name, _)| name == "fix")
                && step.process.is_none()),
        "fix-run needs delivery followed by self-merge in its pull-request worktree"
    );
    steps.into_iter().map(builtin::delivery_step).collect()
}
