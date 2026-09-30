//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped unless
//! it is kept.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::progress;

/// How merging the Base branch, or new commits on origin, into the Issue
/// branch went.
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

    /// Delete the Issue branch on origin, or do nothing if it is already gone
    /// there, e.g. deleted by GitHub after a merge. Hooks are skipped, as for
    /// [`Worktree::push`].
    pub fn delete_from_origin(&self) -> Result<()> {
        progress::step(format_args!("deleting {} on origin", self.branch));
        let Err(error) = self
            .git
            .run(&["push", "--no-verify", "origin", "--delete", &self.branch])
        else {
            return Ok(());
        };
        let reference = format!("refs/heads/{}", self.branch);
        match self.git.run(&["ls-remote", "origin", &reference]) {
            Ok(refs) if refs.is_empty() => Ok(()),
            _ => Err(error),
        }
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

    /// Fetch `origin/<base>` and merge it into the Issue branch.
    pub fn merge_base_branch(&self, base: &str) -> Result<Merge> {
        progress::step(format_args!("merging origin/{base} into {}", self.branch));
        self.git.run(&["fetch", "origin", base])?;
        self.merge(&format!("origin/{base}"))
    }

    /// The Issue branch on origin, as fetched: `origin/<branch>`.
    pub fn upstream(&self) -> String {
        format!("origin/{}", self.branch)
    }

    /// Fetch the Issue branch from origin and list, oldest first, the commits
    /// there that the local Issue branch does not have yet.
    pub fn new_commits_on_origin(&self) -> Result<Vec<String>> {
        self.git.run(&["fetch", "origin", &self.branch])?;
        let range = format!("HEAD..{}", self.upstream());
        let commits = self.git.run(&["rev-list", "--reverse", &range])?;
        Ok(commits.lines().map(String::from).collect())
    }

    /// Fetch the Issue branch from origin and fast-forward the local one to
    /// it, failing if the two have diverged.
    pub fn fast_forward_to_origin(&self) -> Result<()> {
        let upstream = self.upstream();
        progress::step(format_args!("updating {} from {upstream}", self.branch));
        self.git.run(&["fetch", "origin", &self.branch])?;
        self.git
            .run(&["merge", "--ff-only", "--quiet", &upstream])?;
        Ok(())
    }

    /// Merge the Issue branch as last fetched from origin into the local one.
    pub fn merge_new_commits(&self) -> Result<Merge> {
        self.merge(&self.upstream())
    }

    /// Merge `upstream` into the Issue branch: a merge, never a rebase, so
    /// pushing it is always a fast-forward. `--ff` keeps a user's
    /// `merge.ff = only` from turning a clean merge into an error.
    fn merge(&self, upstream: &str) -> Result<Merge> {
        match self.git.run(&["merge", "--no-edit", "--ff", upstream]) {
            Ok(_) => Ok(Merge::Clean),
            Err(_) if self.merge_in_progress()? => Ok(Merge::Conflicted),
            Err(error) => Err(error),
        }
    }

    /// Fail unless `origin/<base>` is fully merged into the Issue branch, e.g.
    /// after a conflict Repair that left the merge unfinished or aborted it.
    pub fn ensure_base_branch_merged(&self, base: &str) -> Result<()> {
        self.ensure_merged(&format!("origin/{base}"))
    }

    /// Like [`Worktree::ensure_base_branch_merged`], for the Issue branch as
    /// last fetched from origin.
    pub fn ensure_new_commits_merged(&self) -> Result<()> {
        self.ensure_merged(&self.upstream())
    }

    fn ensure_merged(&self, upstream: &str) -> Result<()> {
        if self.merge_in_progress()? {
            bail!("the merge of {upstream} is still in progress");
        }
        if !self.merged(upstream)? {
            bail!("{upstream} is not merged into {}", self.branch);
        }
        Ok(())
    }

    /// Fetch `origin/<base>` and say whether it has commits the Issue branch
    /// has not merged yet.
    pub fn base_branch_moved(&self, base: &str) -> Result<bool> {
        self.git.run(&["fetch", "origin", base])?;
        Ok(!self.merged(&format!("origin/{base}"))?)
    }

    /// Whether `upstream`, as last fetched, is fully merged into the Issue
    /// branch.
    fn merged(&self, upstream: &str) -> Result<bool> {
        self.git
            .succeeds(&["merge-base", "--is-ancestor", upstream, "HEAD"])
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
