//! Standalone skill checkouts start at the Launch directory's committed HEAD.

use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::{UntrackedFiles, acquisition, lock_launch, ownership};
use crate::git::Git;
use crate::progress;

pub struct SkillWorktree {
    launch: Git,
    checkout: ownership::Checkout,
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
        Self::checkout(launch, &commit)
    }

    /// A throwaway checkout at `commit`, cleaned up like [`Self::create`].
    pub fn at(launch: &Git, commit: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        Self::checkout(launch, commit)
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
            None => {
                let path = tempfile::Builder::new()
                    .prefix("thirdshift-review-")
                    .tempdir()?
                    .keep();
                acquisition::add_review(launch, branch, &path, commit)?
            }
        };
        Ok(Self {
            launch: Git::new(root),
            checkout,
            untracked: UntrackedFiles::Preserve,
            recovered,
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
            recovered: false,
        })
    }

    /// Author-pass admission is read-only and repeats acquisition's checks.
    pub fn pull_request_conflict(
        launch: &Git,
        branch: &str,
        commit: &str,
    ) -> Result<Option<ReviewConflict>> {
        use std::os::unix::fs::MetadataExt;
        let _lock = lock_launch(launch)?;
        let _refs_lock = launch.lock_worktree_refs()?;
        let entries = acquisition::registrations(launch)?;
        let local = super::local_head(launch, branch)?;
        let reference = format!("refs/heads/{branch}");
        let occupied = entries
            .iter()
            .find(|entry| entry.branch.as_deref() == Some(&reference));
        match acquisition::inspect_review(launch, branch, commit) {
            Ok(_) => Ok(None),
            Err(error) => {
                let mut digest = Sha256::new();
                digest.update(format!("{branch:?}:{local:?}:{commit:?}:{error:#}"));
                if let Some(entry) = occupied {
                    digest.update(format!("{:?}:{}", entry.path, entry.locked));
                    let git = Git::new(&entry.path);
                    for args in [
                        vec![
                            "-c",
                            "core.fsmonitor=false",
                            "status",
                            "--porcelain=v1",
                            "--untracked-files=no",
                        ],
                        vec!["diff", "--no-ext-diff", "--no-textconv", "HEAD", "--"],
                        vec!["diff", "--no-ext-diff", "--no-textconv", "--cached", "--"],
                    ] {
                        digest.update(format!("{:?}", git.run(&args)));
                    }
                    // Identity and operation/evidence changes re-enable notification,
                    // even when refs and the occupied pathname did not move.
                    let admin = git.run(&["rev-parse", "--absolute-git-dir"]).ok();
                    for path in std::iter::once(entry.path.clone())
                        .chain(admin.map(std::path::PathBuf::from))
                    {
                        for name in [
                            "",
                            ".thirdshift-review-token",
                            "thirdshift-review.json",
                            "MERGE_HEAD",
                            "CHERRY_PICK_HEAD",
                            "REVERT_HEAD",
                            "rebase-merge",
                            "rebase-apply",
                            "sequencer",
                        ] {
                            let metadata = std::fs::symlink_metadata(path.join(name));
                            digest.update(format!(
                                "{:?}",
                                metadata.map(|m| (
                                    m.dev(),
                                    m.ino(),
                                    if name.is_empty() {
                                        (0, 0, 0)
                                    } else {
                                        (m.mtime(), m.mtime_nsec(), m.len())
                                    }
                                ))
                            ));
                        }
                    }
                }
                Ok(Some(ReviewConflict {
                    fingerprint: format!("{:x}", digest.finalize()),
                    error,
                }))
            }
        }
    }

    pub fn recovered(&self) -> bool {
        self.recovered
    }

    /// Recommendation retries restart the owned, clean branch at its captured head.
    pub fn restart_recommendation(&self, commit: &str) -> Result<()> {
        if self.recovered {
            self.checkout.ordinary(&self.launch, |_| {
                ownership::restart_retained(&self.launch, &self.checkout, commit)
            })?;
        }
        Ok(())
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
        self.checkout
            .cleanup_preserving_work(&self.launch, self.untracked);
    }
}
