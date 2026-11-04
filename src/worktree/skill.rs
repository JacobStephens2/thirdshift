//! Standalone skill checkouts start at the Launch directory's committed HEAD.

use std::path::Path;

use anyhow::Result;

use super::{UntrackedFiles, acquisition, lock_launch, ownership};
use crate::git::Git;
use crate::progress;

pub struct SkillWorktree {
    launch: Git,
    checkout: ownership::Checkout,
    untracked: UntrackedFiles,
}

impl SkillWorktree {
    /// A throwaway checkout at the Launch directory's committed HEAD.
    pub fn create(launch: &Git) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let commit = launch.run(&["rev-parse", "--verify", "HEAD"])?;
        Self::checkout(launch, &commit)
    }

    /// A throwaway checkout at `commit`, cleaned up like [`Self::create`].
    pub fn at(launch: &Git, commit: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        Self::checkout(launch, commit)
    }

    /// A review uses the author's branch at the captured head. Existing
    /// local work and an already checked-out branch are refused by acquisition.
    pub fn on_pull_request(launch: &Git, branch: &str, commit: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let root = launch.run(&["rev-parse", "--show-toplevel"])?;
        let path = tempfile::Builder::new()
            .prefix("thirdshift-review-")
            .tempdir()?
            .keep();
        let checkout = acquisition::add_review(launch, branch, &path, commit)?;
        Ok(Self {
            launch: Git::new(root),
            checkout,
            untracked: UntrackedFiles::Preserve,
        })
    }

    fn checkout(launch: &Git, commit: &str) -> Result<Self> {
        let root = launch.run(&["rev-parse", "--show-toplevel"])?;
        // Cleanup belongs to the captured checkout, never TempDir's recursive Drop.
        let path = tempfile::Builder::new()
            .prefix("thirdshift-skill-")
            .tempdir()?
            .keep();
        progress::step(format_args!(
            "creating skill worktree {} at {commit}",
            path.display()
        ));
        let checkout = acquisition::add(launch, None, &path, commit)?;
        Ok(Self {
            launch: Git::new(root),
            checkout,
            untracked: UntrackedFiles::Preserve,
        })
    }

    pub fn path(&self) -> &Path {
        self.checkout.path()
    }

    /// A delivered Review run may discard its disposable validation output.
    /// Cleanup still preserves tracked edits and unpushed commits, and verifies
    /// ownership under the disposal locks before removing the acquired checkout.
    pub fn discard_validation_artifacts(&mut self) {
        self.untracked = UntrackedFiles::DiscardValidationArtifacts;
    }
}

impl Drop for SkillWorktree {
    fn drop(&mut self) {
        self.checkout
            .cleanup_preserving_work(&self.launch, self.untracked);
    }
}
