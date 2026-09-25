//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::git::Git;

/// How merging the Base branch into the Issue branch went.
#[derive(Debug, PartialEq, Eq)]
pub enum Merge {
    /// Merged, or nothing to merge.
    Clean,
    /// Conflicted; the merge is left in progress for a conflict Repair.
    Conflicted,
}

pub struct Worktree {
    launch: Git,
    branch: String,
    git: Git,
}

impl Worktree {
    /// Create `branch` fresh from `origin/<base>` in a new worktree next to the
    /// launch repository's root, named `<repo>-<branch>`.
    pub fn create_fresh(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        let root = PathBuf::from(launch.run(&["rev-parse", "--show-toplevel"])?);
        let path = root
            .parent()
            .context("the repository root has no parent directory")?
            .join(format!("{repo}-{branch}"));
        let path_arg = path.to_str().context("worktree path is not UTF-8")?;

        launch.run(&["fetch", "origin", base])?;
        launch.run(&[
            "worktree",
            "add",
            "-b",
            branch,
            path_arg,
            &format!("origin/{base}"),
        ])?;
        Ok(Worktree {
            launch: Git::new(root),
            branch: branch.to_string(),
            git: Git::new(path),
        })
    }

    pub fn path(&self) -> &Path {
        self.git.dir()
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Fetch `origin/<base>` and merge it into the Issue branch: a merge,
    /// never a rebase, so pushing it is always a fast-forward.
    pub fn merge_base(&self, base: &str) -> Result<Merge> {
        self.git.run(&["fetch", "origin", base])?;
        match self
            .git
            .run(&["merge", "--no-edit", &format!("origin/{base}")])
        {
            Ok(_) => Ok(Merge::Clean),
            Err(_) if self.merge_in_progress()? => Ok(Merge::Conflicted),
            Err(error) => Err(error),
        }
    }

    /// Fail if a merge is still in progress, e.g. one a conflict Repair
    /// didn't finish.
    pub fn ensure_no_merge_in_progress(&self, base: &str) -> Result<()> {
        if self.merge_in_progress()? {
            bail!("the merge of origin/{base} is still in progress");
        }
        Ok(())
    }

    fn merge_in_progress(&self) -> Result<bool> {
        self.git
            .succeeds(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.path().to_string_lossy().into_owned();
        // Each step is attempted even if the one before it failed.
        let steps: [&[&str]; 2] = [
            &["worktree", "remove", "--force", &path],
            &["branch", "-D", &self.branch],
        ];
        for step in steps {
            if let Err(error) = self.launch.run(step) {
                eprintln!("thirdshift: cleanup incomplete: {error:#}");
            }
        }
    }
}
