//! Own an add's partial effects while its caller holds the acquisition lock.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::local_head;
use crate::git::Git;

pub(super) fn add(
    launch: &Git,
    branch: Option<&str>,
    path: &Path,
    start: &str,
) -> Result<super::ownership::Checkout> {
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
    match launch
        .run(&args)
        .and_then(|_| super::ownership::Checkout::capture(launch, path, branch))
        .and_then(|checkout| {
            if branch.is_none() {
                owner.publication.publish(launch, &checkout)?;
            }
            Ok(checkout)
        }) {
        Ok(checkout) => {
            owner.armed = false;
            Ok(checkout)
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
    common_dir: PathBuf,
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
        let common_dir = launch.common_dir()?.canonicalize()?;
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
            common_dir,
            publication: super::ownership::Publication::default(),
            armed: false,
        })
    }

    fn recover(&self) -> Vec<String> {
        let launch = self.launch.completion();
        let mut retained = Vec::new();
        let checkout_safe = match self.recover_checkout(&launch) {
            Ok(()) => true,
            Err(error) => {
                retained.push(format!(
                    "retaining checkout {} (expected {} at {}): {error:#}",
                    self.path.display(),
                    self.branch.unwrap_or("detached HEAD"),
                    self.start,
                ));
                false
            }
        };
        if let Some(branch) = self.branch.filter(|_| self.branch_before.is_none())
            && let Err(error) = self.recover_branch(&launch, branch, checkout_safe)
        {
            retained.push(format!(
                "retaining local branch {branch} for {}: {error:#}",
                self.path.display()
            ));
        }
        retained
    }

    fn recover_checkout(&self, launch: &Git) -> Result<()> {
        self.publication.remove_artifacts()?;
        let entries =
            registrations(launch).context("cannot inspect registrations; current head unknown")?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.path == self.registration_path)
        {
            return self.remove_checkout(launch, entry).with_context(|| {
                format!(
                    "on {} at {}",
                    entry.branch.as_deref().unwrap_or(if entry.detached {
                        "detached HEAD"
                    } else {
                        "unknown identity"
                    }),
                    entry.head.as_deref().unwrap_or("unknown head")
                )
            });
        }
        if !self.path_existed && path_exists(self.path)? {
            bail!("unregistered path has uncertain ownership; head unknown");
        }
        Ok(())
    }

    fn remove_checkout(&self, launch: &Git, entry: &Registration) -> Result<()> {
        if self.path_existed || self.registered_before {
            bail!("path or registration existed before this add");
        }
        if entry.locked {
            bail!("checkout is locked");
        }
        let reference = self.branch.map(|branch| format!("refs/heads/{branch}"));
        if entry.branch != reference || entry.detached != self.branch.is_none() {
            bail!("unexpected checkout identity");
        }
        if entry.head.as_deref() != Some(self.start) {
            bail!("checkout HEAD changed from the sampled {}", self.start);
        }
        if std::fs::symlink_metadata(self.path)?
            .file_type()
            .is_symlink()
        {
            bail!("checkout path became a symlink");
        }
        let git = Git::new(self.path).completion();
        if Path::new(&git.run(&["rev-parse", "--show-toplevel"])?) != self.path
            || git.common_dir()?.canonicalize()? != self.common_dir
            || git.run(&["rev-parse", "--verify", "HEAD"])? != self.start
            || git.run(&["rev-parse", "--symbolic-full-name", "HEAD"])?
                != reference.as_deref().unwrap_or("HEAD")
        {
            bail!("checkout no longer matches its registration and sampled start");
        }
        // Status (and non-force removal) can call real edits clean when
        // assume-unchanged or skip-worktree flags hide them. These index
        // entries cannot establish absence of work, even at the sampled HEAD.
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
        launch.run(&[
            "worktree",
            "remove",
            self.path.to_str().context("worktree path is not UTF-8")?,
        ])?;
        Ok(())
    }

    fn recover_branch(&self, launch: &Git, branch: &str, checkout_safe: bool) -> Result<()> {
        let Some(head) =
            local_head(launch, branch).context("cannot inspect local ref; head unknown")?
        else {
            return Ok(());
        };
        if head != self.start {
            bail!("head {head} changed from the sampled {}", self.start);
        }
        if !checkout_safe {
            bail!("head {head}; checkout was retained or could not be inspected");
        }
        let reference = format!("refs/heads/{branch}");
        if let Some(entry) = registrations(launch)
            .with_context(|| format!("head {head}; cannot inspect registered checkouts"))?
            .iter()
            .find(|entry| entry.branch.as_deref() == Some(&reference))
        {
            bail!("head {head}; still registered at {}", entry.path.display());
        }
        // Git locks and compares the old value atomically. Never delete a
        // checked-out ref just because its head happens to match.
        launch
            .run(&["update-ref", "--no-deref", "-d", &reference, self.start])
            .with_context(|| format!("conditional removal failed; last observed head {head}"))?;
        Ok(())
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
