//! Own checked synchronization, preservation and disposal of the acquired instance.

use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{SecondsFormat, Utc};

use super::acquisition::registrations;
use super::{Merge, OriginCommit, PendingMerge, local_head, lock_launch};
use crate::git::Git;
use crate::host;
use crate::process::Interruption;
use crate::progress;

#[path = "review_recovery.rs"]
mod review_recovery;
pub(super) use review_recovery::{Publication, recover as recover_review};

pub(super) struct Checkout {
    root: Directory,
    admin: Directory,
    common: Directory,
    branch: Option<String>,
}

impl Checkout {
    /// Called while acquisition recovery is armed and its lock is held.
    pub(super) fn capture(launch: &Git, path: &Path, branch: Option<&str>) -> Result<Self> {
        let git = Git::new(path).completion();
        let owner = Self {
            root: Directory::open(path.to_path_buf())?,
            admin: Directory::open(PathBuf::from(
                git.run(&["rev-parse", "--absolute-git-dir"])?,
            ))?,
            common: Directory::open(launch.common_dir()?.canonicalize()?)?,
            branch: branch.map(str::to_string),
        };
        owner.inspect(launch)?;
        Ok(owner)
    }

    pub(super) fn path(&self) -> &Path {
        &self.root.path
    }

    /// One interruptible operation owns the lock, including observations and
    /// no-op returns. Fetch takes its refs lock inside this scope, in that order.
    pub(super) fn ordinary<T>(
        &self,
        launch: &Git,
        perform: impl FnOnce(&Operation<'_>) -> Result<T>,
    ) -> Result<T> {
        let operation = Operation::new(self, launch, Purpose::Synchronization);
        operation.interruption.check()?;
        operation.verify_repository()?;
        let lock = lock_launch(launch);
        operation.interruption.check()?;
        let _lock = lock.with_context(|| operation.context.clone())?;
        operation.inspect()?;
        let result = perform(&operation);
        operation.inspect()?;
        result
    }

    fn verify_repository(&self, launch: &Git) -> Result<()> {
        self.common.verify_repository(launch)
    }

    /// A confirmed Self-merge authorizes completion in the captured repository,
    /// independently of checkout attachment, root and administrative identity.
    pub(super) fn delete_from_origin(&self, launch: &Git, branch: &str) -> Result<()> {
        let launch = launch.completion();
        let verify = || {
            self.verify_repository(&launch).with_context(|| {
                format!(
                    "cannot delete expected Issue branch {branch} for acquired checkout {}",
                    self.path().display()
                )
            })
        };
        verify()?;
        let result = launch.run_checked(
            &["push", "--no-verify", "origin", "--delete", branch],
            verify,
        );
        verify()?;
        match result {
            Ok(_) => Ok(()),
            Err(error) => {
                let present = launch.on_origin_checked(branch, verify);
                verify()?;
                match present {
                    Ok(false) => Ok(()),
                    _ => Err(error),
                }
            }
        }
    }

    pub(super) fn preserve_failed_run(&self, launch: &Git, base: &str, reason: &str) -> Result<()> {
        let operation = Operation::new(self, launch, Purpose::Preservation);
        let branch = self
            .branch
            .as_deref()
            .context("Failed run preservation requires an acquired Issue branch")?;
        operation.verify_repository()?;
        let _lock = lock_launch(&operation.launch).with_context(|| operation.context.clone())?;
        let _refs_lock = operation
            .launch
            .lock_worktree_refs()
            .with_context(|| operation.context.clone())?;
        operation.inspect()?;
        if operation.merge_in_progress()? {
            operation.run(&["merge", "--abort"])?;
        }
        operation.run(&["add", "-A"])?;
        let base = format!("origin/{base}");
        if !operation.succeeds(&["diff", "--cached", "--quiet", &base])? {
            let message = format!(
                "thirdshift: failed run ({reason})\n\n\
                 {timestamp}, host {host}. Uncommitted work at the time of failure is included in this commit.",
                timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
                host = host::name().as_deref().unwrap_or("unknown"),
            );
            // No hooks: a hook that rejects the commit would strand the work.
            operation.run(&[
                "commit",
                "-q",
                "--allow-empty",
                "--no-verify",
                "-m",
                &message,
            ])?;
            operation.push(branch)?;
        }
        operation.inspect()?;
        Ok(())
    }

    /// Drop is best effort, including while another panic is unwinding.
    pub(super) fn cleanup(&self, launch: &Git, kept: bool) {
        let launch = launch.completion();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if kept {
                self.retain(&launch, "Failed run preservation did not finish", true);
                return;
            }
            progress::step(match &self.branch {
                Some(branch) => format!("cleaning up the worktree and local branch {branch}"),
                None => "cleaning up the worktree".to_string(),
            });
            if let Err(error) = self.remove(&launch) {
                self.retain(&launch, &format!("{error:#}"), false);
            }
        }));
        if result.is_err() {
            // Avoid invoking Git or the progress recorder again after a panic.
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "thirdshift: warning: retaining checkout {} and {}: cleanup panicked; head unknown",
                self.path().display(),
                self.branch.as_deref().unwrap_or("detached HEAD"),
            );
        }
    }

    fn retain(&self, launch: &Git, reason: &str, kept: bool) {
        let reason = reason.replace('\n', "; ");
        let head = Git::new(self.path())
            .completion()
            .run(&["rev-parse", "--verify", "HEAD"])
            .ok()
            .or_else(|| {
                self.branch
                    .as_deref()
                    .and_then(|branch| local_head(launch, branch).ok().flatten())
            });
        progress::step(format_args!(
            "{} {} and {} at {}: {reason}",
            if kept {
                "keeping the worktree"
            } else {
                "warning: retaining checkout"
            },
            self.path().display(),
            self.branch
                .as_ref()
                .map(|branch| format!("local branch {branch}"))
                .as_deref()
                .unwrap_or("detached HEAD"),
            head.as_deref().unwrap_or("unknown head"),
        ));
    }

    fn remove(&self, launch: &Git) -> Result<()> {
        self.verify_repository(launch)?;
        let _lock = lock_launch(launch).context("cannot establish cleanup worktree lock")?;
        self.remove_locked(launch, || Ok(()))
    }

    /// Stale recovery already holds the worktree lock. Its durable evidence
    /// supplies additional authority; live disposal uses lifetime identity.
    fn remove_locked(&self, launch: &Git, evidence: impl Fn() -> Result<()>) -> Result<()> {
        self.verify_repository(launch)?;
        let _refs_lock = launch.lock_worktree_refs()?;
        let disposal = Disposal::capture(self, launch, &evidence)?;
        disposal.remove().with_context(|| {
            format!(
                "disposal stopped at captured cleanup head {}",
                disposal.head
            )
        })
    }

    /// Directory handles distinguish recreation even at the same paths and HEAD.
    /// The current commit is observed here, never pinned to the acquisition HEAD.
    pub(super) fn inspect(&self, launch: &Git) -> Result<String> {
        self.root.verify().context("checkout directory identity")?;
        self.admin
            .verify()
            .context("administrative directory identity")?;
        self.verify_repository(launch)?;
        let entries = registrations(launch)?;
        let entry = entries
            .iter()
            .find(|entry| entry.path == self.root.path)
            .context("original checkout registration is missing")?;
        if entry.locked {
            bail!("checkout registration is locked");
        }
        let reference = self
            .branch
            .as_ref()
            .map(|branch| format!("refs/heads/{branch}"));
        if entry.branch != reference || entry.detached != self.branch.is_none() {
            bail!("attached/detached checkout identity changed");
        }
        if fs::symlink_metadata(self.path().join(".git"))?
            .file_type()
            .is_symlink()
        {
            bail!("checkout Git link became a symlink");
        }
        if fs::symlink_metadata(self.admin.path.join("gitdir"))?
            .file_type()
            .is_symlink()
        {
            bail!("administrative Git backlink became a symlink");
        }
        if fs::symlink_metadata(self.admin.path.join("commondir"))?
            .file_type()
            .is_symlink()
        {
            bail!("administrative common repository link became a symlink");
        }
        let git = Git::new(self.path()).completion();
        let head = git.run(&["rev-parse", "--verify", "HEAD"])?;
        let backlink = fs::read_to_string(self.admin.path.join("gitdir"))?;
        if Path::new(&git.run(&["rev-parse", "--show-toplevel"])?) != self.root.path
            || Path::new(&git.run(&["rev-parse", "--absolute-git-dir"])?) != self.admin.path
            || git.common_dir()?.canonicalize()? != self.common.path
            || Path::new(backlink.trim_end()).canonicalize()? != self.path().join(".git")
            || git.run(&["rev-parse", "--symbolic-full-name", "HEAD"])?
                != reference.as_deref().unwrap_or("HEAD")
            || entry.head.as_deref() != Some(&head)
        {
            bail!("checkout and registration ownership evidence is inconsistent");
        }
        if let Some(branch) = &self.branch
            && local_head(launch, branch)?.as_deref() != Some(&head)
        {
            bail!("attached Issue ref does not match checkout HEAD");
        }
        // Recheck after Git observations, before permitting a destructive step.
        self.root.verify()?;
        self.admin.verify()?;
        self.verify_repository(launch)?;
        Ok(head)
    }

    pub(super) fn verify_removed(
        &self,
        launch: &Git,
    ) -> Result<Vec<super::acquisition::Registration>> {
        self.verify_repository(launch)?;
        for path in [&self.root.path, &self.admin.path] {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("cannot inspect removed path {}", path.display())
                    });
                }
                Ok(_) => bail!(
                    "removed path {} exists; remaining resources retained",
                    path.display()
                ),
            }
        }
        let entries = registrations(launch)?;
        if entries.iter().any(|entry| entry.path == self.root.path) {
            bail!("removed checkout registration still exists");
        }
        self.verify_repository(launch)?;
        Ok(entries)
    }
}

/// One acquired disposal owns the head and exact configuration snapshot
/// through checkout, ref and configuration removal, under both caller locks.
struct Disposal<'a> {
    checkout: &'a Checkout,
    launch: &'a Git,
    evidence: &'a dyn Fn() -> Result<()>,
    head: String,
    config: Vec<String>,
}

impl<'a> Disposal<'a> {
    fn capture(
        checkout: &'a Checkout,
        launch: &'a Git,
        evidence: &'a dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        let head = checkout.inspect(launch)?;
        let config = checkout
            .branch
            .as_deref()
            .map(|branch| branch_config(launch, branch))
            .transpose()?
            .unwrap_or_default();
        let disposal = Self {
            checkout,
            launch,
            evidence,
            head,
            config,
        };
        disposal.verify_checkout()?;
        Ok(disposal)
    }

    fn remove(&self) -> Result<()> {
        self.launch
            .run_transition(
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.checkout
                        .path()
                        .to_str()
                        .context("worktree path is not UTF-8")?,
                ],
                || self.verify_checkout(),
                |result| {
                    if result.as_ref().is_ok_and(|output| output.status.success()) {
                        self.checkout.verify_removed(self.launch).map(|_| ())
                    } else {
                        self.verify_checkout()
                    }
                },
            )
            .context("owned checkout removal failed; local branch retained")?;
        if let Some(branch) = &self.checkout.branch {
            self.remove_branch(branch)?;
        }
        Ok(())
    }

    fn verify_checkout(&self) -> Result<()> {
        if self.checkout.inspect(self.launch)? != self.head {
            bail!("checkout head changed since cleanup snapshot");
        }
        (self.evidence)()?;
        self.checkout.verify_repository(self.launch)
    }

    fn verify_branch(&self, branch: &str, expected: Option<&str>) -> Result<()> {
        let entries = self.checkout.verify_removed(self.launch)?;
        let reference = format!("refs/heads/{branch}");
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.branch.as_deref() == Some(&reference))
        {
            bail!("local branch still registered at {}", entry.path.display());
        }
        let head = local_head(self.launch, branch)?;
        if head.as_deref() != expected {
            if expected.is_none() {
                bail!("local ref was recreated; branch configuration retained");
            }
            bail!(
                "local branch head changed since cleanup snapshot (expected {}, observed {})",
                self.head,
                head.as_deref().unwrap_or("absent")
            );
        }
        self.checkout.verify_repository(self.launch)
    }

    fn remove_branch(&self, branch: &str) -> Result<()> {
        let launch = self.launch;
        let head = &self.head;
        let reference = format!("refs/heads/{branch}");
        launch
            .run_transition(
                &["update-ref", "--no-deref", "-d", &reference, head],
                || self.verify_branch(branch, Some(head)),
                |result| {
                    let expected = if result.as_ref().is_ok_and(|output| output.status.success()) {
                        None
                    } else {
                        Some(head.as_str())
                    };
                    self.verify_branch(branch, expected)
                },
            )
            .with_context(|| format!("conditional removal failed; last observed head {head}"))?;
        self.remove_configuration(branch)
    }

    fn verify_configuration(&self, branch: &str, removed: bool) -> Result<()> {
        self.verify_branch(branch, None)?;
        // Exact records preserve subsection case, duplicate order, embedded
        // newlines, and valueless versus empty configuration.
        let config = branch_config(self.launch, branch)?;
        if if removed {
            !config.is_empty()
        } else {
            config != self.config
        } {
            bail!("branch configuration changed since cleanup snapshot; configuration retained");
        }
        self.verify_branch(branch, None)
    }

    fn remove_configuration(&self, branch: &str) -> Result<()> {
        if self.config.is_empty() {
            return self.verify_configuration(branch, false);
        }
        self.launch.run_transition(
            &[
                "config",
                "--local",
                "--no-includes",
                "--remove-section",
                &format!("branch.{branch}"),
            ],
            || self.verify_configuration(branch, false),
            |result| {
                self.verify_configuration(
                    branch,
                    result.as_ref().is_ok_and(|output| output.status.success()),
                )
            },
        )?;
        Ok(())
    }
}

/// Git execution cannot escape this acquired operation. Inspection before and
/// after every command gates later commands and even successful no-op results.
/// Advisory locks coordinate factory operations, not arbitrary outside writers.
pub(super) struct Operation<'a> {
    checkout: &'a Checkout,
    launch: Git,
    git: Git,
    interruption: Interruption,
    context: String,
}

enum Purpose {
    Synchronization,
    Preservation,
}

impl<'a> Operation<'a> {
    /// Bind command execution, inspection interruption and diagnostics together.
    /// Inspection's Git observations finish before the interruption recheck,
    /// as they do for acquisition and disposal, without clearing the request.
    fn new(checkout: &'a Checkout, launch: &Git, purpose: Purpose) -> Self {
        let git = Git::new(checkout.path());
        let (git, interruption, action) = match purpose {
            Purpose::Synchronization => (git, Interruption::Ordinary, "synchronize"),
            Purpose::Preservation => (git.completion(), Interruption::Completion, "preserve"),
        };
        Self {
            checkout,
            launch: launch.completion(),
            git,
            interruption,
            context: format!(
                "cannot {action} acquired checkout {} on expected Issue branch {}",
                checkout.path().display(),
                checkout.branch.as_deref().unwrap_or("unknown"),
            ),
        }
    }

    fn verify_repository(&self) -> Result<()> {
        self.checkout
            .verify_repository(&self.launch)
            .with_context(|| self.context.clone())
    }

    fn inspect(&self) -> Result<String> {
        self.interruption.check()?;
        let result = self
            .checkout
            .inspect(&self.launch)
            .with_context(|| self.context.clone());
        self.interruption.check()?;
        result
    }

    fn execute<T>(&self, command: impl FnOnce(&Git) -> Result<T>) -> Result<T> {
        self.inspect()?;
        let result = command(&self.git);
        self.inspect()?;
        result
    }

    pub(super) fn run(&self, args: &[&str]) -> Result<String> {
        self.git.run_checked(args, || self.inspect().map(|_| ()))
    }

    pub(super) fn head(&self) -> Result<String> {
        self.inspect()
    }

    pub(super) fn push(&self, branch: &str) -> Result<()> {
        self.git.push_checked(branch, || self.inspect().map(|_| ()))
    }

    pub(super) fn sample_origin(&self, branch: &str) -> Result<OriginCommit> {
        self.git
            .fetch_checked(&[branch], || self.inspect().map(|_| ()))?;
        super::fetched_origin(branch, |args| self.run(args))
    }

    pub(super) fn merge_in_progress(&self) -> Result<bool> {
        self.execute(Git::merge_in_progress)
    }

    pub(super) fn merged(&self, rev: &str) -> Result<bool> {
        self.succeeds(&["merge-base", "--is-ancestor", rev, "HEAD"])
    }

    /// Boolean probes are gated but never retried or reinterpreted as errors
    /// for completed nonzero exits (including an absent comparison ref).
    fn succeeds(&self, args: &[&str]) -> Result<bool> {
        self.execute(|git| git.succeeds(args))
    }

    /// Merge the exact sampled commit, preserving the readable upstream label
    /// and allowing a merge even when the repository config demands ff-only.
    pub(super) fn merge(&self, origin: OriginCommit, branch: &str) -> Result<Merge> {
        let message = format!(
            "Merge remote-tracking branch '{}' into {branch}",
            origin.upstream
        );
        match self.run(&["merge", "--no-edit", "--ff", "-m", &message, &origin.commit]) {
            Ok(_) => Ok(Merge::Clean {
                commit: origin.commit,
            }),
            Err(_) if self.merge_in_progress()? => Ok(Merge::Conflicted(PendingMerge { origin })),
            Err(error) => Err(error),
        }
    }
}

pub(super) struct Directory {
    path: PathBuf,
    handle: File,
}

impl Directory {
    pub(super) fn open(path: PathBuf) -> Result<Self> {
        let handle = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(&path)
            .with_context(|| format!("cannot pin directory {}", path.display()))?;
        let directory = Self { path, handle };
        directory.verify()?;
        Ok(directory)
    }

    pub(super) fn verify_repository(&self, launch: &Git) -> Result<()> {
        self.verify().context("common repository identity")?;
        if launch.common_dir()?.canonicalize()? != self.path {
            bail!("launch no longer belongs to the captured common repository");
        }
        self.verify()
    }

    fn verify(&self) -> Result<()> {
        let current = fs::symlink_metadata(&self.path).with_context(|| {
            format!("cannot inspect original directory {}", self.path.display())
        })?;
        let pinned = self.handle.metadata()?;
        if !current.is_dir() || current.dev() != pinned.dev() || current.ino() != pinned.ino() {
            bail!(
                "directory {} was replaced or became a symlink",
                self.path.display()
            );
        }
        Ok(())
    }
}

/// Full local listing preserves inspection errors, unlike a get-regexp probe.
/// Includes are disabled so only the exact local subsection may be removed.
fn branch_config(launch: &Git, branch: &str) -> Result<Vec<String>> {
    let listing = launch.run(&["config", "--local", "--no-includes", "--null", "--list"])?;
    if listing.contains('\u{fffd}') || (!listing.is_empty() && !listing.ends_with('\0')) {
        bail!("cannot establish an unambiguous branch configuration snapshot");
    }
    let section = format!("branch.{branch}");
    Ok(listing
        .split('\0')
        .filter(|record| {
            let key = record.split_once('\n').map_or(*record, |(key, _)| key);
            key.rsplit_once('.')
                .is_some_and(|(prefix, _)| prefix == section)
        })
        .map(str::to_string)
        .collect())
}
