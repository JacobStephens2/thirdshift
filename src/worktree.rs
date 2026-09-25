//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped unless
//! it is kept.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::progress;

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
    /// Leave the worktree and local Issue branch in place when dropped.
    kept: bool,
}

impl Worktree {
    /// Create `branch` fresh from `origin/<base>` in a new worktree next to the
    /// launch repository's root, named `<repo>-<branch>`.
    pub fn create_fresh(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        launch.run(&["fetch", "origin", base])?;
        Self::add(
            launch,
            repo,
            branch,
            &["-b", branch],
            &format!("origin/{base}"),
        )
    }

    /// Check out the existing `branch` from origin in a new worktree next to
    /// the launch repository's root, named `<repo>-<branch>`. `origin/<base>`
    /// is fetched too, for the review fixed point. A local `branch`, if any, is
    /// reset to origin's: `branch::select` has checked they already match.
    pub fn continue_existing(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        launch.run(&["fetch", "origin", base, branch])?;
        Self::add(
            launch,
            repo,
            branch,
            &["-B", branch],
            &format!("origin/{branch}"),
        )
    }

    fn add(
        launch: &Git,
        repo: &str,
        branch: &str,
        branch_args: &[&str],
        start: &str,
    ) -> Result<Self> {
        let root = PathBuf::from(launch.run(&["rev-parse", "--show-toplevel"])?);
        let path = root
            .parent()
            .context("the repository root has no parent directory")?
            .join(format!("{repo}-{branch}"));
        let path_arg = path.to_str().context("worktree path is not UTF-8")?;

        progress::step(format_args!(
            "creating worktree {} on {branch} from {start}",
            path.display()
        ));
        let mut args = vec!["worktree", "add"];
        args.extend_from_slice(branch_args);
        args.extend([path_arg, start]);
        launch.run(&args)?;
        Ok(Worktree {
            launch: Git::new(root),
            branch: branch.to_string(),
            git: Git::new(path),
            kept: false,
        })
    }

    pub fn path(&self) -> &Path {
        self.git.dir()
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Push the Issue branch to origin (a no-op if it is already there).
    /// The target repo's hooks are skipped: the session runs the tests and CI
    /// gates the PR, so a local hook doesn't decide whether work reaches
    /// origin.
    pub fn push(&self) -> Result<()> {
        progress::step(format_args!("pushing {}", self.branch));
        self.git
            .run(&["push", "--no-verify", "origin", &self.branch])?;
        Ok(())
    }

    /// Let go of the worktree without removing it or the local Issue branch,
    /// for work that may exist nowhere else. Dropping it then says where they
    /// are and the branch's head commit.
    pub fn keep(mut self) {
        self.kept = true;
    }

    /// The Issue branch's head commit.
    pub fn head(&self) -> Result<String> {
        self.git.run(&["rev-parse", "HEAD"])
    }

    /// Fetch `origin/<base>` and merge it into the Issue branch: a merge,
    /// never a rebase, so pushing it is always a fast-forward. `--ff` keeps a
    /// user's `merge.ff = only` from turning a clean merge into an error.
    pub fn merge_base_branch(&self, base: &str) -> Result<Merge> {
        progress::step(format_args!("merging origin/{base} into {}", self.branch));
        self.git.run(&["fetch", "origin", base])?;
        match self
            .git
            .run(&["merge", "--no-edit", "--ff", &format!("origin/{base}")])
        {
            Ok(_) => Ok(Merge::Clean),
            Err(_) if self.merge_in_progress()? => Ok(Merge::Conflicted),
            Err(error) => Err(error),
        }
    }

    /// Fail unless `origin/<base>` is fully merged into the Issue branch, e.g.
    /// after a conflict Repair that left the merge unfinished or aborted it.
    pub fn ensure_base_branch_merged(&self, base: &str) -> Result<()> {
        if self.merge_in_progress()? {
            bail!("the merge of origin/{base} is still in progress");
        }
        if !self.base_branch_merged(base)? {
            bail!("origin/{base} is not merged into {}", self.branch);
        }
        Ok(())
    }

    /// Fetch `origin/<base>` and say whether it has commits the Issue branch
    /// has not merged yet.
    pub fn base_branch_moved(&self, base: &str) -> Result<bool> {
        self.git.run(&["fetch", "origin", base])?;
        Ok(!self.base_branch_merged(base)?)
    }

    /// Whether the fetched `origin/<base>` is fully merged into the Issue
    /// branch.
    fn base_branch_merged(&self, base: &str) -> Result<bool> {
        let upstream = format!("origin/{base}");
        self.git
            .succeeds(&["merge-base", "--is-ancestor", &upstream, "HEAD"])
    }

    /// Whether a merge is in progress in the worktree.
    pub fn merge_in_progress(&self) -> Result<bool> {
        self.git
            .succeeds(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.path().to_string_lossy().into_owned();
        if self.kept {
            let head = self
                .head()
                .unwrap_or_else(|error| format!("an unknown commit ({error:#})"));
            progress::step(format_args!(
                "keeping the worktree {path} and local branch {} at {head}",
                self.branch
            ));
            return;
        }
        progress::step(format_args!(
            "cleaning up the worktree and local branch {}",
            self.branch
        ));
        // Each step is attempted even if the one before it failed.
        let steps: [&[&str]; 2] = [
            &["worktree", "remove", "--force", &path],
            &["branch", "-D", &self.branch],
        ];
        for step in steps {
            if let Err(error) = self.launch.run(step) {
                progress::step(format_args!("cleanup incomplete: {error:#}"));
            }
        }
    }
}
