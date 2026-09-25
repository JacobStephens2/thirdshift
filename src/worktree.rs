//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::git::Git;
use crate::progress;

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

        progress::step(format_args!(
            "creating worktree {} on {branch} from origin/{base}",
            path.display()
        ));
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
                eprintln!("thirdshift: cleanup incomplete: {error:#}");
            }
        }
    }
}
