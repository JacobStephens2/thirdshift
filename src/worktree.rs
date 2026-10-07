//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped unless
//! Failed run preservation fails. And the Architecture review's: a sibling
//! too, on no branch, always removed when dropped.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{SecondsFormat, Utc};

use crate::git::Git;
use crate::host;
use crate::progress;

#[cfg(test)]
mod acquisition_tests;
#[cfg(test)]
mod preservation_tests;

/// How merging the Base branch, or new commits on origin, into the Issue
/// branch went.
#[derive(Debug)]
pub enum Merge {
    /// Merged, or nothing to merge.
    Clean,
    /// Conflicted; the merge is left in progress for a conflict Repair.
    Conflicted(PendingMerge),
}

/// A conflicted merge left for a conflict Repair: `upstream` as it was when
/// the merge began, pinned to `commit` because another Run's fetch may move
/// the shared remote-tracking ref before the Repair finishes.
#[derive(Debug)]
pub struct PendingMerge {
    upstream: String,
    commit: String,
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
    /// launch repository's root, named `<repo>-<branch>`. Under the worktree
    /// lock, pin the fetched Base branch commit, refuse any existing local
    /// Issue branch and create without force. No selection preflight is required.
    pub fn create_fresh(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        launch.run(&["fetch", "origin", base])?;
        let source = format!("origin/{base}");
        let commit = launch.run(&["rev-parse", &format!("refs/remotes/{source}")])?;
        check_local_branch(branch, local_head(launch, branch)?.as_deref(), None)?;
        Self::add(launch, repo, branch, &["-b", branch], &commit, &source)
    }

    /// Check out the existing `branch` from origin in a new worktree next to
    /// the launch repository's root, named `<repo>-<branch>`. `origin/<base>`
    /// is fetched too, for the review fixed point. Under the worktree lock,
    /// pin the fetched head and refuse any different local head. An equal
    /// branch is attached without resetting it; an absent one is created
    /// without force at the pinned commit. No selection preflight is required.
    pub fn continue_existing(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        launch.run(&["fetch", "origin", base, branch])?;
        let source = format!("origin/{branch}");
        let commit = launch.run(&["rev-parse", &format!("refs/remotes/{source}")])?;
        let local = local_head(launch, branch)?;
        check_local_branch(branch, local.as_deref(), Some(&commit))?;
        if local.is_some() {
            Self::add(launch, repo, branch, &[], branch, &source)
        } else {
            Self::add(launch, repo, branch, &["-b", branch], &commit, &source)
        }
    }

    fn add(
        launch: &Git,
        repo: &str,
        branch: &str,
        branch_args: &[&str],
        start: &str,
        source: &str,
    ) -> Result<Self> {
        let (root, path) = sibling(launch, &format!("{repo}-{branch}"))?;
        progress::step(format_args!(
            "creating worktree {} on {branch} from {source}",
            path.display()
        ));
        add_worktree(launch, branch_args, &path, start)?;
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

    /// The Launch directory the worktree was added from.
    pub fn launch(&self) -> &Git {
        &self.launch
    }

    /// Push the Issue branch to origin (a no-op if it is already there).
    /// The target repo's hooks are skipped: the session runs the tests and CI
    /// gates the PR, so a local hook doesn't decide whether work reaches
    /// origin.
    pub fn push(&self) -> Result<()> {
        self.git.push(&self.branch)
    }

    /// Preserve all work from a Failed run against the last-fetched Base
    /// branch, even after Command interruption. An unfinished merge is
    /// aborted first; changes are committed as the failure marker and pushed
    /// without local hooks. With no changes, nothing is committed or pushed.
    /// Retention is armed before salvage: any error or unwinding leaves the
    /// worktree and local Issue branch in place. Success allows normal cleanup.
    pub fn preserve_failed_run(&mut self, base: &str, reason: &str) -> Result<()> {
        self.kept = true;
        let git = self.git.completion();
        if git.merge_in_progress()? {
            git.run(&["merge", "--abort"])?;
        }
        git.run(&["add", "-A"])?;
        let base = format!("origin/{base}");
        if !git.succeeds(&["diff", "--cached", "--quiet", &base])? {
            let message = format!(
                "thirdshift: failed run ({reason})\n\n\
                 {timestamp}, host {host}. Uncommitted work at the time of failure is included in this commit.",
                timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                host = host::name().as_deref().unwrap_or("unknown"),
            );
            // No hooks: a hook that rejects the commit would strand the work.
            git.run(&[
                "commit",
                "-q",
                "--allow-empty",
                "--no-verify",
                "-m",
                &message,
            ])?;
            git.push(&self.branch)?;
        }
        self.kept = false;
        Ok(())
    }

    /// Delete the Issue branch on origin, or do nothing if it is already gone
    /// there, e.g. deleted by GitHub after a merge. Hooks are skipped, as for
    /// [`Worktree::push`]. This finishes a confirmed Self-merge, including
    /// the origin read that reconciles a failed deletion.
    pub fn delete_from_origin(&self) -> Result<()> {
        progress::step(format_args!("deleting {} on origin", self.branch));
        let git = self.git.completion();
        let Err(error) = git.run(&["push", "--no-verify", "origin", "--delete", &self.branch])
        else {
            return Ok(());
        };
        match git.on_origin(&self.branch) {
            Ok(false) => Ok(()),
            _ => Err(error),
        }
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

    /// The Base branch commit the Issue branch last merged in: the newest
    /// commit of `origin/<base>`, as last fetched, that its head contains.
    /// Another Run's fetch may have moved the shared remote-tracking ref on
    /// since the merge, which this is unaffected by.
    pub fn merged_base_commit(&self, base: &str) -> Result<String> {
        self.git
            .run(&["merge-base", "HEAD", &format!("origin/{base}")])
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
            Err(_) if self.git.merge_in_progress()? => Ok(Merge::Conflicted(PendingMerge {
                upstream: upstream.to_string(),
                commit: self.git.run(&["rev-parse", "MERGE_HEAD"])?,
            })),
            Err(error) => Err(error),
        }
    }

    /// Fail unless the commit of a conflicted merge is merged into the Issue
    /// branch, e.g. after a conflict Repair that left the merge unfinished or
    /// aborted it. The upstream may have moved on since; merging that is the
    /// next round's work.
    pub fn ensure_merged(&self, pending: &PendingMerge) -> Result<()> {
        let upstream = &pending.upstream;
        if self.git.merge_in_progress()? {
            bail!("the merge of {upstream} is still in progress");
        }
        if !self.merged(&pending.commit)? {
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

    /// Whether `rev` is fully merged into the Issue branch.
    fn merged(&self, rev: &str) -> Result<bool> {
        self.git
            .succeeds(&["merge-base", "--is-ancestor", rev, "HEAD"])
    }
}

/// The local Issue branch head sampled for acquisition or advisory preflight.
pub(crate) fn local_head(launch: &Git, branch: &str) -> Result<Option<String>> {
    launch.run_optional(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ])
}

/// Refuse local work acquisition would replace. A fresh checkout requires
/// absence; a Continuation permits absence or exact equality with its
/// sampled origin head. Selection shares this rule for advisory preflight;
/// acquisition checks it again under the worktree lock after fetching.
pub(crate) fn check_local_branch(
    branch: &str,
    local_head: Option<&str>,
    origin_head: Option<&str>,
) -> Result<()> {
    let Some(local_head) = local_head else {
        return Ok(());
    };
    match origin_head {
        Some(origin_head) if origin_head == local_head => Ok(()),
        Some(_) => bail!(
            "the local branch {branch} differs from origin/{branch}; push, reset or delete it first"
        ),
        None => {
            bail!("the local branch {branch} is not on origin; push, rename or delete it first")
        }
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        let path = self.path().to_string_lossy().into_owned();
        if self.kept {
            let head = self
                .git
                .completion()
                .run(&["rev-parse", "HEAD"])
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
        let launch = self.launch.completion();
        let _lock = lock_launch(&launch).inspect_err(|error| {
            progress::step(format_args!("cleaning up without the lock: {error:#}"))
        });
        // Each step is attempted even if the one before it failed.
        let steps: [&[&str]; 2] = [
            &["worktree", "remove", "--force", &path],
            &["branch", "-D", &self.branch],
        ];
        for step in steps {
            if let Err(error) = launch.run(step) {
                progress::step(format_args!("cleanup incomplete: {error:#}"));
            }
        }
    }
}

/// The worktree an Architecture review runs in: detached at the head of the
/// Base branch on origin, with no Issue branch, and removed when dropped,
/// whatever the session left in it. Nothing in it is committed or pushed.
pub struct ReviewWorktree {
    launch: Git,
    path: PathBuf,
}

impl ReviewWorktree {
    /// Check out `origin/<base>`, detached, in a new worktree next to the
    /// launch repository's root, named `<repo>-architect`. A worktree left
    /// there by a process that ended before it could remove it is removed
    /// first, so the caller sees that no Architecture review is still running
    /// in it.
    pub fn create(launch: &Git, repo: &str, base: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        launch.run(&["fetch", "origin", base])?;
        let (root, path) = sibling(launch, &format!("{repo}-architect"))?;
        if path.exists() {
            let stale = path.to_str().context("worktree path is not UTF-8")?;
            // Anything else at the path is left for `git worktree add` to
            // refuse.
            if launch.succeeds(&["worktree", "remove", "--force", stale])? {
                progress::step(format_args!("removed the leftover worktree {stale}"));
            }
        }
        let start = format!("origin/{base}");
        progress::step(format_args!(
            "creating worktree {} detached at {start}",
            path.display()
        ));
        add_worktree(launch, &["--detach"], &path, &start)?;
        Ok(ReviewWorktree {
            launch: Git::new(root),
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ReviewWorktree {
    fn drop(&mut self) {
        progress::step("cleaning up the worktree");
        let launch = self.launch.completion();
        let _lock = lock_launch(&launch).inspect_err(|error| {
            progress::step(format_args!("cleaning up without the lock: {error:#}"))
        });
        let path = self.path.to_string_lossy();
        if let Err(error) = launch.run(&["worktree", "remove", "--force", &path]) {
            progress::step(format_args!("cleanup incomplete: {error:#}"));
        }
    }
}

/// The launch repository's root, and the path of the worktree `name` next to
/// it.
fn sibling(launch: &Git, name: &str) -> Result<(PathBuf, PathBuf)> {
    let root = PathBuf::from(launch.run(&["rev-parse", "--show-toplevel"])?);
    let path = root
        .parent()
        .context("the repository root has no parent directory")?
        .join(name);
    Ok((root, path))
}

/// `git worktree add <checkout> <path> <start>` in the launch repository,
/// where `checkout` says what the worktree is on: a branch, or nothing.
fn add_worktree(launch: &Git, checkout: &[&str], path: &Path, start: &str) -> Result<()> {
    let path = path.to_str().context("worktree path is not UTF-8")?;
    let mut args = vec!["worktree", "add"];
    args.extend_from_slice(checkout);
    args.extend([path, start]);
    launch.run(&args)?;
    Ok(())
}

/// Wait for, then hold until the file is dropped, the Launch directory's
/// worktree lock, so the Runs of a Spec run's Tickets add and remove their
/// worktrees and local Issue branches one at a time: `git worktree add -b`
/// and `git branch -D` can fail partway on a lock file another holds.
fn lock_launch(launch: &Git) -> Result<File> {
    launch.lock("thirdshift-worktrees.lock")
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    /// A clone `work` of a bare `origin.git` with one commit on `main`, both
    /// in a temp directory. No global or system config is read.
    pub(super) fn launch_directory() -> (tempfile::TempDir, Git) {
        let temp = tempfile::TempDir::new().unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(
            temp.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        git(temp.path(), &["clone", "-q", "origin.git", "work"]);
        let work = temp.path().join("work");
        let hooks = temp.path().join("hooks");
        std::fs::create_dir(&hooks).unwrap();
        git(
            &temp.path().join("origin.git"),
            &[
                "config",
                "core.hooksPath",
                temp.path().join("origin.git/hooks").to_str().unwrap(),
            ],
        );
        for (key, value) in [
            ("user.name", "Test Runner"),
            ("user.email", "runner@example.com"),
            ("commit.gpgSign", "false"),
            ("core.hooksPath", hooks.to_str().unwrap()),
        ] {
            git(&work, &["config", key, value]);
        }
        git(
            &work,
            &[
                "-c",
                "user.name=Test Runner",
                "-c",
                "user.email=runner@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "Initial",
            ],
        );
        git(&work, &["push", "-q", "origin", "HEAD:main"]);
        (temp, Git::new(work))
    }

    #[test]
    fn a_worktree_is_created_only_once_another_run_lets_go_of_the_launch_directory() {
        let (temp, launch) = launch_directory();
        let held = lock_launch(&launch).unwrap();
        let path = temp.path().join("work-issue-1");

        let creating = std::thread::spawn(move || {
            Worktree::create_fresh(&launch, "work", "issue-1", "main").unwrap()
        });
        std::thread::sleep(Duration::from_millis(300));
        let created_while_held = path.exists();
        drop(held);
        let worktree = creating.join().unwrap();

        assert!(!created_while_held);
        assert_eq!(worktree.path(), path.canonicalize().unwrap());
    }
}
