//! Standalone skill checkouts start at the Launch directory's committed HEAD.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::Result;

use super::{UntrackedFiles, acquisition, lock_launch, ownership};
use crate::git::Git;
use crate::progress;

pub struct SkillWorktree {
    launch: Git,
    checkout: Option<ownership::Checkout>,
    untracked: UntrackedFiles,
    recovered: bool,
}

/// A local conflict's private fingerprint contains no worktree contents.
pub struct ReviewConflict {
    pub fingerprint: String,
    pub error: anyhow::Error,
}

impl SkillWorktree {
    /// A throwaway checkout at the Launch directory's committed HEAD.
    pub fn create(launch: &Git) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let commit = launch.run(&["rev-parse", "--verify", "HEAD"])?;
        Self::create_at(launch, &commit)
    }

    /// A throwaway checkout at `commit`, cleaned up like [`Self::create`].
    pub fn at(launch: &Git, commit: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        Self::create_at(launch, commit)
    }

    /// A review uses the author's branch at the captured head. Existing
    /// local work is refused unless durable evidence authorizes clean recovery.
    pub fn on_pull_request(launch: &Git, branch: &str, commit: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let root = launch.run(&["rev-parse", "--show-toplevel"])?;
        let retained = acquisition::recover_pull_request(launch, branch, commit)?;
        let recovered = retained.is_some();
        let checkout = match retained {
            Some(checkout) => checkout,
            None => acquire_in_temp("thirdshift-review-", |path| {
                acquisition::add_review(launch, branch, path, commit)
            })?,
        };
        Ok(Self {
            launch: Git::new(root),
            checkout: Some(checkout),
            // Even an undelivered Review run's named validation output is disposable.
            untracked: UntrackedFiles::DiscardNamedValidationArtifacts,
            recovered,
        })
    }

    fn create_at(launch: &Git, commit: &str) -> Result<Self> {
        let root = launch.run(&["rev-parse", "--show-toplevel"])?;
        let checkout = acquire_in_temp("thirdshift-skill-", |path| {
            progress::step(format_args!(
                "creating skill worktree {} at {commit}",
                path.display()
            ));
            acquisition::add(launch, None, path, commit)
        })?;
        Ok(Self {
            launch: Git::new(root),
            checkout: Some(checkout),
            untracked: UntrackedFiles::Preserve,
            recovered: false,
        })
    }

    /// Author-pass admission is read-only and repeats acquisition's checks.
    pub fn pull_request_conflict(
        launch: &Git,
        branch: &str,
        commit: &str,
    ) -> Result<Option<ReviewConflict>> {
        let _lock = lock_launch(launch)?;
        let _refs_lock = launch.lock_worktree_refs()?;
        ownership::pull_request_conflict(launch, branch, commit)
    }

    pub fn recovered(&self) -> bool {
        self.recovered
    }

    /// Recommendation retries restart the owned, clean branch at its captured head.
    pub fn restart_recommendation(&self, commit: &str) -> Result<()> {
        if self.recovered {
            self.checkout().ordinary(&self.launch, |_| {
                ownership::restart_retained(&self.launch, self.checkout(), commit)
            })?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        self.checkout().path()
    }

    fn checkout(&self) -> &ownership::Checkout {
        self.checkout
            .as_ref()
            .expect("checkout not handed to Delivery")
    }

    /// Transfer the acquired instance to the shared Repair loop without
    /// reacquiring origin, which could silently adopt an unreviewed head.
    pub(crate) fn take_for_delivery(&mut self, branch: &str) -> super::Worktree {
        super::Worktree {
            failed_work: super::FailedWork::Fix,
            checkout: self.checkout.take().expect("checkout handed over once"),
            launch: Git::new(self.launch.dir()),
            branch: branch.into(),
            synchronization_owner: std::sync::Arc::new(()),
            cleanup: super::Cleanup::Remove,
        }
    }

    /// A delivered Review run may discard all of its untracked validation
    /// output, not only the named entries every Review checkout may discard.
    /// Cleanup still preserves tracked edits and unpushed commits, and verifies
    /// ownership under the disposal locks before removing the acquired checkout.
    pub fn discard_validation_artifacts(&mut self) {
        self.untracked = UntrackedFiles::DiscardAllUntracked;
    }

    /// Observe only the acquired checkout, even if an agent replaced its path.
    pub fn head(&self) -> Result<String> {
        self.checkout()
            .ordinary(&self.launch, |operation| operation.head())
    }

    /// Compare with the PR's captured base commit, rather than a moving ref.
    pub fn merge_base_commit(&self, base_commit: &str) -> Result<String> {
        self.checkout().ordinary(&self.launch, |operation| {
            operation.run(&["merge-base", base_commit, "HEAD"])
        })
    }
}

/// Cleanup belongs to the captured checkout, never TempDir's recursive Drop:
/// a failed acquisition can retain work there. Failure removes only the
/// allocated directory itself, still empty, and keeps acquisition's error.
fn acquire_in_temp(
    prefix: &str,
    acquire: impl FnOnce(&Path) -> Result<ownership::Checkout>,
) -> Result<ownership::Checkout> {
    let directory = tempfile::Builder::new().prefix(prefix).tempdir()?;
    let allocated = std::fs::symlink_metadata(directory.path())?;
    let path = directory.keep();
    acquire(&path).inspect_err(|_| {
        // remove_dir refuses a directory with contents, such as a retained checkout.
        if std::fs::symlink_metadata(&path).is_ok_and(|current| {
            current.is_dir() && current.dev() == allocated.dev() && current.ino() == allocated.ino()
        }) {
            let _ = std::fs::remove_dir(&path);
        }
    })
}

impl Drop for SkillWorktree {
    fn drop(&mut self) {
        if let Some(checkout) = &self.checkout {
            checkout.cleanup_preserving_work(&self.launch, self.untracked);
        }
    }
}
