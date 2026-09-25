//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::git::Git;

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
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.path().to_string_lossy().into_owned();
        let removed = self
            .launch
            .run(&["worktree", "remove", "--force", &path])
            .and_then(|_| self.launch.run(&["branch", "-D", &self.branch]));
        if let Err(error) = removed {
            eprintln!("thirdshift: cleanup incomplete: {error:#}");
        }
    }
}
