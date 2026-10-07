//! The Run's worktree: a sibling of the launch repository, on the Issue
//! branch, removed together with the local Issue branch when dropped unless
//! Failed run preservation fails or ownership is uncertain. And the
//! Architecture review's: a sibling too, on no branch, disposable when owned.
//! Both retain the acquired instance's identity and leave replacements alone.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::progress;

mod acquisition;
#[cfg(test)]
mod acquisition_tests;
#[cfg(test)]
mod cleanup_tests;
mod ownership;
#[cfg(test)]
mod preservation_tests;
#[cfg(test)]
mod review_recovery_tests;
#[cfg(test)]
mod synchronization_tests;

/// How merging the Base branch, or new commits on origin, into the Issue
/// branch went.
#[derive(Debug)]
pub enum Merge<P = PendingMerge> {
    /// Merged, or nothing to merge, with the selected origin commit.
    Clean { commit: String },
    /// Conflicted; the merge is left in progress for a conflict Repair.
    Conflicted(P),
}

/// A conflicted merge left for a conflict Repair: `upstream` as it was when
/// the merge began, pinned to `commit` because another Run's fetch may move
/// the shared remote-tracking ref before the Repair finishes.
#[derive(Debug)]
pub struct PendingMerge {
    origin: OriginCommit,
}

/// One remote-tracking ref sample, with its readable label kept beside the ID.
#[derive(Debug)]
struct OriginCommit {
    upstream: String,
    commit: String,
}

/// A read-only observation of Foreign commits. Its facts share one origin
/// sample and one local head. It can be consumed once, by the Worktree that
/// produced it, while that local head is unchanged.
#[derive(Debug)]
pub struct ForeignCommits {
    owner: Arc<()>,
    origin: OriginCommit,
    local_head: String,
    commits: Vec<String>,
}

impl ForeignCommits {
    pub fn origin_head(&self) -> &str {
        &self.origin.commit
    }

    pub fn local_head(&self) -> &str {
        &self.local_head
    }

    pub fn upstream(&self) -> &str {
        &self.origin.upstream
    }

    /// Oldest first, the commits missing from the observed local head.
    pub fn commits(&self) -> &[String] {
        &self.commits
    }

    /// A coherent linear batch for the scripted Delivery adapter only.
    #[cfg(test)]
    pub(crate) fn fixture(local_head: &str, commits: &[&str]) -> Self {
        Self {
            owner: Arc::new(()),
            origin: OriginCommit {
                commit: commits.last().copied().unwrap_or(local_head).to_string(),
                upstream: "origin/issue-7".to_string(),
            },
            local_head: local_head.to_string(),
            commits: commits.iter().map(|sha| sha.to_string()).collect(),
        }
    }
}

pub struct Worktree {
    checkout: ownership::Checkout,
    launch: Git,
    branch: String,
    /// Per-instance identity: a later worktree at the same path must not
    /// consume this one's observations. No lock is held by an observation.
    synchronization_owner: Arc<()>,
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
        launch.fetch(&[base])?;
        let origin = fetched_origin(base, |args| launch.run(args))?;
        check_local_branch(branch, local_head(launch, branch)?.as_deref(), None)?;
        Self::add(launch, repo, branch, &origin.commit, &origin.upstream)
    }

    /// Check out the existing `branch` from origin in a new worktree next to
    /// the launch repository's root, named `<repo>-<branch>`. `origin/<base>`
    /// is fetched too, for the review fixed point. Under the worktree lock,
    /// pin the fetched head and refuse any different local head. An equal
    /// branch is attached without resetting it; an absent one is created
    /// without force at the pinned commit. No selection preflight is required.
    pub fn continue_existing(launch: &Git, repo: &str, branch: &str, base: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        launch.fetch(&[base, branch])?;
        let origin = fetched_origin(branch, |args| launch.run(args))?;
        let local = local_head(launch, branch)?;
        check_local_branch(branch, local.as_deref(), Some(&origin.commit))?;
        Self::add(launch, repo, branch, &origin.commit, &origin.upstream)
    }

    fn add(launch: &Git, repo: &str, branch: &str, start: &str, source: &str) -> Result<Self> {
        let (root, path) = sibling(launch, &format!("{repo}-{branch}"))?;
        progress::step(format_args!(
            "creating worktree {} on {branch} from {source}",
            path.display()
        ));
        let checkout = acquisition::add(launch, Some(branch), &path, start)?;
        Ok(Worktree {
            checkout,
            launch: Git::new(root),
            branch: branch.to_string(),
            synchronization_owner: Arc::new(()),
            kept: false,
        })
    }

    pub fn path(&self) -> &Path {
        self.checkout.path()
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
        self.checkout
            .ordinary(&self.launch, |operation| operation.push(&self.branch))
    }

    /// Preserve all work from a Failed run against the last-fetched Base
    /// branch, even after Command interruption. An unfinished merge is
    /// aborted first; changes are committed as the failure marker and pushed
    /// without local hooks. With no changes, nothing is committed or pushed.
    /// Only the acquired checkout instance on its Issue branch may be changed.
    /// Retention is armed before salvage: any error or unwinding leaves the
    /// worktree and local Issue branch in place. Final verified ownership
    /// allows normal cleanup.
    pub fn preserve_failed_run(&mut self, base: &str, reason: &str) -> Result<()> {
        self.kept = true;
        self.checkout
            .preserve_failed_run(&self.launch, base, reason)?;
        self.kept = false;
        Ok(())
    }

    /// Delete the Issue branch on origin, or do nothing if it is already gone
    /// there, e.g. deleted by GitHub after a merge. Hooks are skipped, as for
    /// [`Worktree::push`]. This finishes a confirmed Self-merge, including
    /// the origin read that reconciles a failed deletion, through the captured
    /// Launch repository even if the acquired checkout is no longer valid.
    pub fn delete_from_origin(&self) -> Result<()> {
        progress::step(format_args!("deleting {} on origin", self.branch));
        self.checkout.delete_from_origin(&self.launch, &self.branch)
    }

    /// The Issue branch's head commit.
    pub fn head(&self) -> Result<String> {
        self.checkout
            .ordinary(&self.launch, |operation| operation.head())
    }

    /// Fetch and sample `origin/<base>`, then merge that commit into the
    /// Issue branch. A clean outcome includes the selected commit for CI
    /// comparison, including when the merge had nothing to do.
    pub fn merge_base_branch(&self, base: &str) -> Result<Merge> {
        progress::step(format_args!("merging origin/{base} into {}", self.branch));
        self.checkout.ordinary(&self.launch, |operation| {
            operation.merge(operation.sample_origin(base)?, &self.branch)
        })
    }

    /// Fetch the Issue branch from origin and list, oldest first, the commits
    /// there that the local Issue branch does not have yet, together with
    /// the exact heads and readable upstream label. Does not change the
    /// local head; a caller may refuse the batch before merging it.
    pub fn new_commits_on_origin(&self) -> Result<ForeignCommits> {
        self.checkout.ordinary(&self.launch, |operation| {
            let origin = operation.sample_origin(&self.branch)?;
            let local_head = operation.head()?;
            let mut observed = ForeignCommits {
                owner: Arc::clone(&self.synchronization_owner),
                origin,
                local_head,
                commits: Vec::new(),
            };
            let range = format!("{}..{}", observed.local_head(), observed.origin_head());
            observed.commits = operation
                .run(&["rev-list", "--reverse", &range])?
                .lines()
                .map(String::from)
                .collect();
            Ok(observed)
        })
    }

    /// Fetch the Issue branch from origin and fast-forward the local one to
    /// it, failing if the two have diverged.
    pub fn fast_forward_to_origin(&self) -> Result<()> {
        progress::step(format_args!(
            "updating {} from origin/{}",
            self.branch, self.branch
        ));
        self.checkout.ordinary(&self.launch, |operation| {
            let origin = operation.sample_origin(&self.branch)?;
            operation.run(&["merge", "--ff-only", "--quiet", &origin.commit])?;
            Ok(())
        })
    }

    /// Consume this Worktree's observation, merging its sampled origin head.
    /// Refuse before mutation if it belongs to another worktree or the local
    /// head has changed since observation.
    pub fn merge_new_commits(&self, observed: ForeignCommits) -> Result<Merge> {
        self.checkout.ordinary(&self.launch, |operation| {
            if !Arc::ptr_eq(&observed.owner, &self.synchronization_owner) {
                bail!("the Foreign commit observation belongs to another worktree");
            }
            if operation.head()? != observed.local_head() {
                bail!(
                    "the local head changed since observing {}",
                    observed.upstream()
                );
            }
            operation.merge(observed.origin, &self.branch)
        })
    }

    /// Fail unless the commit of a conflicted merge is merged into the Issue
    /// branch, e.g. after a conflict Repair that left the merge unfinished or
    /// aborted it. The upstream may have moved on since; merging that is the
    /// next round's work.
    pub fn ensure_merged(&self, pending: &PendingMerge) -> Result<()> {
        self.checkout.ordinary(&self.launch, |operation| {
            let upstream = &pending.origin.upstream;
            if operation.merge_in_progress()? {
                bail!("the merge of {upstream} is still in progress");
            }
            if !operation.merged(&pending.origin.commit)? {
                bail!("{upstream} is not merged into {}", self.branch);
            }
            Ok(())
        })
    }

    /// Fetch `origin/<base>` and say whether it has commits the Issue branch
    /// has not merged yet.
    pub fn base_branch_moved(&self, base: &str) -> Result<bool> {
        self.checkout.ordinary(&self.launch, |operation| {
            let origin = operation.sample_origin(base)?;
            Ok(!operation.merged(&origin.commit)?)
        })
    }
}

/// The local Issue branch head sampled for acquisition or advisory preflight.
pub(crate) fn local_head(launch: &Git, branch: &str) -> Result<Option<String>> {
    let reference = format!("refs/heads/{branch}");
    let refs = launch.run(&[
        "for-each-ref",
        "--format=%(refname)%00%(objectname)%00%(symref)",
        &reference,
    ])?;
    for line in refs.lines() {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.first() == Some(&reference.as_str()) {
            if fields.len() != 3 || !fields[2].is_empty() {
                bail!("cannot establish the local branch {branch}: unexpected ref identity");
            }
            return Ok(Some(fields[1].to_string()));
        }
    }
    Ok(None)
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
        self.checkout.cleanup(&self.launch, self.kept);
    }
}

/// The worktree an Architecture review runs in: detached at the head of the
/// Base branch on origin, with no Issue branch, and removed when dropped,
/// whatever the session left in the owned instance. Replaced or uncertain
/// resources are retained. Nothing in it is committed or pushed by cleanup.
pub struct ReviewWorktree {
    launch: Git,
    checkout: ownership::Checkout,
}

impl ReviewWorktree {
    /// Check out `origin/<base>`, detached, in a new worktree next to the
    /// launch repository's root, named `<repo>-architect`. A worktree left
    /// there by a process that ended before cleanup is removed only with
    /// evidence of successful acquisition and unchanged instance ownership.
    /// Failed, unmarked and uncertain acquisitions are retained and named.
    pub fn create(launch: &Git, repo: &str, base: &str) -> Result<Self> {
        let _lock = lock_launch(launch)?;
        let (root, path) = sibling(launch, &format!("{repo}-architect"))?;
        ownership::recover_review(launch, &path)?;
        launch.fetch(&[base])?;
        let origin = fetched_origin(base, |args| launch.run(args))?;
        progress::step(format_args!(
            "creating worktree {} detached at {}",
            path.display(),
            origin.upstream,
        ));
        let checkout = acquisition::add(launch, None, &path, &origin.commit)?;
        Ok(ReviewWorktree {
            launch: Git::new(root),
            checkout,
        })
    }

    pub fn path(&self) -> &Path {
        self.checkout.path()
    }
}

impl Drop for ReviewWorktree {
    fn drop(&mut self) {
        self.checkout.cleanup(&self.launch, false);
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

/// Pin an already-fetched origin branch, avoiding ambiguous local names.
fn fetched_origin(
    branch: &str,
    run: impl FnOnce(&[&str]) -> Result<String>,
) -> Result<OriginCommit> {
    let upstream = format!("origin/{branch}");
    let commit = run(&[
        "rev-parse",
        "--verify",
        &format!("refs/remotes/{upstream}^{{commit}}"),
    ])?;
    Ok(OriginCommit { upstream, commit })
}

/// Wait for, then hold until the file is dropped, the Launch directory's
/// worktree lock, so the Runs of a Spec run's Tickets add and remove their
/// worktrees and local Issue branches one at a time. Acquisition and verified
/// cleanup hold it through their filesystem, registration and ref effects.
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
