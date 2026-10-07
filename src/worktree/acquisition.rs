//! Own an add's partial effects while its caller holds the acquisition lock.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::local_head;
use super::ownership::{Checkout, Directory};
use crate::git::Git;

pub(super) fn add(
    launch: &Git,
    branch: Option<&str>,
    path: &Path,
    start: &str,
) -> Result<super::ownership::Checkout> {
    let _refs_lock = launch.lock_worktree_refs()?;
    let mut owner = Acquisition::inspect(launch, branch, path, start)?;
    let mut args = vec!["worktree", "add"];
    match branch {
        Some(branch) if owner.branch_before.is_none() => args.extend(["-b", branch]),
        Some(_) => {}
        None => args.push("--detach"),
    }
    args.extend([
        path.to_str().context("worktree path is not UTF-8")?,
        if owner.branch_before.is_some() {
            // Worktree add attaches only a short local branch name; a
            // qualified ref makes Git detach instead. Inspections use the
            // qualified ref and the pinned commit throughout.
            branch.context("missing Issue branch")?
        } else {
            start
        },
    ]);
    // Arm before the first possible mutation, including unwinding in Git.
    owner.armed = true;
    match launch.run(&args).and_then(|_| {
        owner.checkout = Some(Checkout::capture(launch, path, branch)?);
        if branch.is_none() {
            owner.publication.publish(
                launch,
                owner
                    .checkout
                    .as_ref()
                    .context("missing captured checkout")?,
            )?;
        }
        Ok(())
    }) {
        Ok(()) => {
            owner.armed = false;
            owner.checkout.take().context("missing captured checkout")
        }
        Err(cause) => {
            let retained = owner.recover();
            owner.armed = false;
            if retained.is_empty() {
                Err(cause)
            } else {
                Err(AcquisitionFailure { cause, retained }.into())
            }
        }
    }
}

struct Acquisition<'a> {
    launch: &'a Git,
    path: &'a Path,
    branch: Option<&'a str>,
    start: &'a str,
    path_existed: bool,
    registration_path: PathBuf,
    registered_before: bool,
    branch_before: Option<String>,
    common: Directory,
    checkout: Option<Checkout>,
    publication: super::ownership::Publication,
    armed: bool,
}

impl<'a> Acquisition<'a> {
    fn inspect(
        launch: &'a Git,
        branch: Option<&'a str>,
        path: &'a Path,
        start: &'a str,
    ) -> Result<Self> {
        path.to_str().context("worktree path is not UTF-8")?;
        let common = Directory::open(launch.common_dir()?.canonicalize()?)?;
        let path_existed = path_exists(path)?;
        let registration_path = if path_existed {
            path.canonicalize()
                .with_context(|| format!("cannot resolve {} before adding", path.display()))?
        } else {
            path.to_path_buf()
        };
        let registered_before = registrations(launch)?
            .iter()
            .any(|entry| entry.path == registration_path);
        let branch_before = branch
            .map(|branch| local_head(launch, branch))
            .transpose()?
            .flatten();
        common.verify_repository(launch)?;
        if let Some(branch) = branch {
            super::check_local_branch(branch, branch_before.as_deref(), Some(start))?;
        }
        Ok(Self {
            launch,
            path,
            branch,
            start,
            path_existed,
            registration_path,
            registered_before,
            branch_before,
            common,
            checkout: None,
            publication: super::ownership::Publication::default(),
            armed: false,
        })
    }

    fn recover(&mut self) -> Vec<String> {
        let launch = self.launch.completion();
        Recovery {
            acquisition: self,
            launch,
        }
        .run()
    }
}

/// Recovery alone owns clean-only authority and the verified checkout outcome.
/// Both caller locks remain held; outside writers still require fresh checks.
struct Recovery<'a, 'repo> {
    acquisition: &'a mut Acquisition<'repo>,
    launch: Git,
}

#[derive(Clone, Copy)]
enum CheckoutOutcome {
    NoEffect,
    Removed,
}

impl Recovery<'_, '_> {
    fn run(&mut self) -> Vec<String> {
        let mut retained = Vec::new();
        let outcome = match self.recover_checkout() {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                let owner = &self.acquisition;
                retained.push(format!(
                    "retaining checkout {} (expected {} at {}): {error:#}",
                    owner.path.display(),
                    owner.branch.unwrap_or("detached HEAD"),
                    owner.start,
                ));
                None
            }
        };
        let owner = &self.acquisition;
        if let Some(branch) = owner.branch.filter(|_| owner.branch_before.is_none())
            && let Err(error) = self.recover_branch(branch, outcome)
        {
            retained.push(format!(
                "retaining local branch {branch} for {} (expected head {}): {error:#}",
                owner.path.display(),
                owner.start,
            ));
        }
        retained
    }

    fn verify_repository(&self) -> Result<()> {
        self.acquisition.common.verify_repository(&self.launch)
    }

    fn recover_checkout(&mut self) -> Result<CheckoutOutcome> {
        self.verify_repository().context("current head unknown")?;
        self.acquisition.publication.remove_artifacts()?;
        let entries = registrations(&self.launch)
            .context("cannot inspect registrations; current head unknown")?;
        self.verify_repository()?;
        let owner = &mut self.acquisition;
        let Some(entry) = entries
            .iter()
            .find(|entry| entry.path == owner.registration_path)
        else {
            self.verify_outcome(CheckoutOutcome::NoEffect)?;
            return Ok(CheckoutOutcome::NoEffect);
        };
        let identity = format!(
            "on {} at {}",
            entry.branch.as_deref().unwrap_or(if entry.detached {
                "detached HEAD"
            } else {
                "unknown identity"
            }),
            entry.head.as_deref().unwrap_or("unknown head"),
        );
        let mut capture = || -> Result<()> {
            if owner.path_existed || owner.registered_before {
                bail!("path or registration existed before this add");
            }
            if entry.locked {
                bail!("checkout is locked");
            }
            let reference = owner.branch.map(|branch| format!("refs/heads/{branch}"));
            if entry.branch != reference || entry.detached != owner.branch.is_none() {
                bail!("unexpected checkout identity");
            }
            if entry.head.as_deref() != Some(owner.start) {
                bail!("checkout HEAD changed from the sampled {}", owner.start);
            }
            if owner.checkout.is_none() {
                owner.checkout = Some(Checkout::capture(&self.launch, owner.path, owner.branch)?);
            }
            Ok(())
        };
        capture().context(identity.clone())?;
        self.launch
            .run_transition(
                &[
                    "worktree",
                    "remove",
                    self.acquisition
                        .path
                        .to_str()
                        .context("worktree path is not UTF-8")?,
                ],
                || self.verify_clean_checkout(),
                |result| {
                    if result.as_ref().is_ok_and(|output| output.status.success()) {
                        self.verify_outcome(CheckoutOutcome::Removed).map(|_| ())
                    } else {
                        self.verify_clean_checkout()
                    }
                },
            )
            .context(identity)?;
        Ok(CheckoutOutcome::Removed)
    }

    fn verify_clean_checkout(&self) -> Result<()> {
        self.verify_repository()?;
        let owner = &self.acquisition;
        let checkout = owner
            .checkout
            .as_ref()
            .context("checkout identity unknown")?;
        let head = checkout.inspect(&self.launch)?;
        if head != owner.start {
            bail!(
                "checkout HEAD changed from the sampled {} to {head}",
                owner.start
            );
        }
        let git = Git::new(owner.path).completion();
        // Hidden index entries cannot establish absence of tracked work.
        if git
            .run(&["-c", "core.fsmonitor=false", "ls-files", "-v", "-z"])?
            .split('\0')
            .any(|file| {
                file.as_bytes()
                    .first()
                    .is_some_and(|flag| flag.is_ascii_lowercase() || *flag == b'S')
            })
        {
            bail!("checkout has uncertain index flags that can hide tracked work");
        }
        if !git
            .run(&[
                "-c",
                "core.fsmonitor=false",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=matching",
                "--ignore-submodules=none",
            ])?
            .is_empty()
        {
            bail!("checkout contains work");
        }
        if git
            .run(&["submodule", "status", "--recursive"])?
            .lines()
            .any(|line| !line.starts_with('-'))
        {
            bail!("checkout contains initialized submodules");
        }
        let head = checkout.inspect(&self.launch)?;
        if head != owner.start {
            bail!(
                "checkout HEAD changed from the sampled {} to {head}",
                owner.start
            );
        }
        self.verify_repository()
    }

    fn verify_outcome(&self, outcome: CheckoutOutcome) -> Result<Vec<Registration>> {
        self.verify_repository()?;
        let owner = &self.acquisition;
        let entries = match outcome {
            CheckoutOutcome::NoEffect => {
                if owner.checkout.is_some() || owner.registered_before {
                    bail!("original checkout registration disappeared; head unknown");
                }
                let entries = registrations(&self.launch)?;
                if entries
                    .iter()
                    .any(|entry| entry.path == owner.registration_path)
                {
                    bail!("unexpected checkout registration appeared; head unknown");
                }
                if !owner.path_existed && path_exists(owner.path)? {
                    bail!("unregistered path has uncertain ownership; head unknown");
                }
                entries
            }
            CheckoutOutcome::Removed => owner
                .checkout
                .as_ref()
                .context("removed checkout identity unknown")?
                .verify_removed(&self.launch)?,
        };
        self.verify_repository()?;
        Ok(entries)
    }

    fn recover_branch(&self, branch: &str, outcome: Option<CheckoutOutcome>) -> Result<()> {
        self.verify_repository().context("current head unknown")?;
        let Some(head) =
            local_head(&self.launch, branch).context("cannot inspect local ref; head unknown")?
        else {
            return self.verify_repository();
        };
        if head != self.acquisition.start {
            bail!(
                "head {head} changed from the sampled {}",
                self.acquisition.start
            );
        }
        let outcome = outcome.with_context(|| {
            format!("head {head}; checkout was retained or could not be inspected")
        })?;
        let reference = format!("refs/heads/{branch}");
        self.launch
            .run_transition(
                &[
                    "update-ref",
                    "--no-deref",
                    "-d",
                    &reference,
                    self.acquisition.start,
                ],
                || self.verify_branch(branch, outcome, Some(self.acquisition.start)),
                |result| {
                    let expected = if result.as_ref().is_ok_and(|output| output.status.success()) {
                        None
                    } else {
                        Some(self.acquisition.start)
                    };
                    self.verify_branch(branch, outcome, expected)
                },
            )
            .with_context(|| format!("conditional removal failed; last observed head {head}"))?;
        Ok(())
    }

    fn verify_branch(
        &self,
        branch: &str,
        outcome: CheckoutOutcome,
        expected: Option<&str>,
    ) -> Result<()> {
        if self.acquisition.branch_before.is_some() {
            bail!("Issue branch existed before this add");
        }
        let unused = || -> Result<()> {
            let entries = self.verify_outcome(outcome)?;
            let reference = format!("refs/heads/{branch}");
            if let Some(entry) = entries
                .iter()
                .find(|entry| entry.branch.as_deref() == Some(&reference))
            {
                bail!(
                    "head {}; still registered at {}",
                    entry.head.as_deref().unwrap_or("unknown"),
                    entry.path.display()
                );
            }
            Ok(())
        };
        unused()?;
        let head =
            local_head(&self.launch, branch).context("cannot inspect local ref; head unknown")?;
        if head.as_deref() != expected {
            bail!(
                "local branch head changed (expected {}, observed {})",
                expected.unwrap_or("absent"),
                head.as_deref().unwrap_or("absent")
            );
        }
        // Ref observations must not authorize recreated paths or new attachments.
        unused()?;
        self.verify_repository()
    }
}

impl Drop for Acquisition<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Remain inside the caller's lock, use completion execution, and
        // never turn an acquisition panic into a second panic.
        let recovered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for message in self.recover() {
                let _ = writeln!(std::io::stderr(), "thirdshift: {message}");
            }
        }));
        if recovered.is_err() {
            let _ = writeln!(
                std::io::stderr(),
                "thirdshift: retaining checkout {} and {}: recovery panicked; current head unknown (expected {})",
                self.path.display(),
                self.branch.unwrap_or("detached HEAD"),
                self.start
            );
        }
    }
}

#[derive(Debug)]
struct AcquisitionFailure {
    cause: anyhow::Error,
    retained: Vec<String>,
}

impl std::fmt::Display for AcquisitionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\n{}", self.cause, self.retained.join("\n"))
    }
}

impl std::error::Error for AcquisitionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

fn path_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("cannot inspect {}", path.display())),
    }
}

pub(super) struct Registration {
    pub(super) path: PathBuf,
    pub(super) head: Option<String>,
    pub(super) branch: Option<String>,
    pub(super) detached: bool,
    pub(super) locked: bool,
}

pub(super) fn registrations(launch: &Git) -> Result<Vec<Registration>> {
    let listing = launch.run(&["worktree", "list", "--porcelain", "-z"])?;
    if listing.contains('\u{fffd}') {
        bail!("worktree registration is not unambiguous UTF-8; head unknown");
    }
    let entries: Vec<_> =
        listing
            .split("\0\0")
            .filter(|record| !record.is_empty())
            .map(|record| {
                let mut fields = record.split('\0');
                let path = fields
                    .next()
                    .and_then(|field| field.strip_prefix("worktree "))
                    .context("cannot inspect worktree registration; head unknown")?;
                let mut entry = Registration {
                    path: PathBuf::from(path),
                    head: None,
                    branch: None,
                    detached: false,
                    locked: false,
                };
                let mut bare = false;
                for field in fields {
                    if let Some(head) = field.strip_prefix("HEAD ")
                        && entry.head.replace(head.to_string()).is_some()
                    {
                        bail!("cannot inspect registration: duplicate HEAD");
                    }
                    if let Some(branch) = field.strip_prefix("branch ")
                        && entry.branch.replace(branch.to_string()).is_some()
                    {
                        bail!("cannot inspect registration: duplicate branch");
                    }
                    entry.detached |= field == "detached";
                    entry.locked |= field == "locked" || field.starts_with("locked ");
                    bare |= field == "bare";
                }
                if !bare
                    && (entry.head.as_deref().is_none_or(|head| {
                        head.is_empty() || head.bytes().all(|byte| byte == b'0')
                    }) || entry.branch.is_some() == entry.detached)
                {
                    bail!(
                        "cannot inspect registration at {}: checkout identity or head unknown",
                        entry.path.display()
                    );
                }
                Ok(entry)
            })
            .collect::<Result<_>>()?;
    let mut paths = std::collections::HashSet::new();
    for entry in &entries {
        if !paths.insert(&entry.path) {
            bail!(
                "cannot inspect duplicate registration at {}",
                entry.path.display()
            );
        }
    }
    // Git omits damaged administrative directories (e.g. a missing gitdir).
    // An incomplete listing cannot prove that a branch is unused.
    let admin = launch.common_dir()?.join("worktrees");
    let count = match std::fs::read_dir(&admin) {
        Ok(dirs) => {
            let mut count = 0;
            for dir in dirs {
                let dir = dir?;
                if !dir.file_type()?.is_dir() {
                    bail!(
                        "cannot inspect worktree administration at {}",
                        dir.path().display()
                    );
                }
                count += 1;
            }
            count
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", admin.display()));
        }
    };
    if entries.len() != count + 1 {
        bail!(
            "cannot inspect every registration in {}; checkout path and head unknown",
            admin.display()
        );
    }
    Ok(entries)
}
