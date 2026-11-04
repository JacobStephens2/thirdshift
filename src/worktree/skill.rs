//! Standalone skill checkouts start at the Launch directory's committed HEAD.

use std::path::Path;

use anyhow::Result;

use super::{acquisition, lock_launch, ownership};
use crate::git::Git;
use crate::progress;

pub struct SkillWorktree {
    launch: Git,
    checkout: ownership::Checkout,
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
        })
    }

    pub fn path(&self) -> &Path {
        self.checkout.path()
    }

    /// Observe only the acquired checkout, even if an agent replaced its path.
    pub fn head(&self) -> Result<String> {
        self.checkout
            .ordinary(&self.launch, |operation| operation.head())
    }

    /// Compare with the PR's captured base commit, rather than a moving ref.
    pub fn merge_base_commit(&self, base_commit: &str) -> Result<String> {
        self.checkout.ordinary(&self.launch, |operation| {
            operation.run(&["merge-base", base_commit, "HEAD"])
        })
    }
}

impl Drop for SkillWorktree {
    fn drop(&mut self) {
        self.checkout.cleanup_preserving_work(&self.launch);
    }
}
