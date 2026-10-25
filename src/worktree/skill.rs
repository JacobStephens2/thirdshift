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
    pub fn create(launch: &Git) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let root = launch.run(&["rev-parse", "--show-toplevel"])?;
        let commit = launch.run(&["rev-parse", "--verify", "HEAD"])?;
        // Cleanup belongs to the captured checkout, never TempDir's recursive Drop.
        let path = tempfile::Builder::new()
            .prefix("thirdshift-skill-")
            .tempdir()?
            .keep();
        progress::step(format_args!(
            "creating skill worktree {} at {commit}",
            path.display()
        ));
        let checkout = acquisition::add(launch, None, &path, &commit)?;
        Ok(Self {
            launch: Git::new(root),
            checkout,
        })
    }

    pub fn path(&self) -> &Path {
        self.checkout.path()
    }
}

impl Drop for SkillWorktree {
    fn drop(&mut self) {
        self.checkout.cleanup_skill(&self.launch);
    }
}
