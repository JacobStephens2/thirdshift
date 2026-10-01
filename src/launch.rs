//! The start of a run with no Issue URL, an Architect run or a Pickup run:
//! its repository is the one the Launch directory's `origin` names, its Base
//! branch the one its command named or else the one checked out there. Both
//! make the same preflight checks, then try the same lock, so only one of
//! them per repository runs at a time on a machine.

use std::fmt;
use std::fs::{self, File, TryLockError};

use anyhow::{Context, Result};

use crate::config;
use crate::git::Git;
use crate::issue::Repo;
use crate::preflight;

/// The Launch directory of a run with no Issue URL, its checks passed and
/// its repository's lock held.
pub struct Launch {
    pub git: Git,
    /// The URL of its `origin`.
    pub origin: String,
    /// The GitHub repository `origin` names.
    pub repo: Repo,
    /// The branch checked out there, or none on a detached HEAD.
    pub checked_out: Option<String>,
    /// The run's Base branch.
    pub base: String,
}

/// Why a run with no Issue URL is skipped at its start: another on the
/// repository, an Architect run or a Pickup run, is still running on this
/// machine, the Spec run or Run it dispatched included. Its `Display` is the
/// reason, as the skipped run gives it.
#[derive(Debug)]
pub struct AlreadyRunning(pub Repo);

impl fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "an Architect run or a Pickup run is already running on {}",
            self.0.slug()
        )
    }
}

/// Start a run with no Issue URL from the Launch directory, the current
/// directory. Its Base branch is `base`, the branch its command named,
/// whatever the Launch directory has checked out, or without one the branch
/// checked out there; on a detached HEAD with neither, the failure says to
/// check out the branch `detached_advice` describes, as in "the branch the
/// work should be based on".
///
/// Once the preflight checks pass, the run is [`AlreadyRunning`], with
/// nothing done, if another on the same repository is still running on this
/// machine. Otherwise this process is that repository's one such run until it
/// exits, through whatever it dispatches.
pub fn start(base: Option<&str>, detached_advice: &str) -> Result<Result<Launch, AlreadyRunning>> {
    let (git, origin, repo) = directory()?;
    preflight::check_identity(&git)?;
    let checked_out = git
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok();
    let base = base
        .or(checked_out.as_deref())
        .with_context(|| {
            format!("HEAD is detached; check out {detached_advice}, or name it with base <branch>")
        })?
        .to_string();
    preflight::check_base_branch(&git, &base)?;
    let Some(lock) = try_run_lock(&repo)? else {
        return Ok(Err(AlreadyRunning(repo)));
    };
    // Never closed, so the lock is held for as long as this process lives,
    // through the Spec run or Run it dispatches, and the operating system
    // releases it however the process ends.
    std::mem::forget(lock);
    Ok(Ok(Launch {
        git,
        origin,
        repo,
        checked_out,
        base,
    }))
}

/// The Launch directory, the URL of its `origin`, and the GitHub repository
/// that names.
fn directory() -> Result<(Git, String, Repo)> {
    let git = Git::new(std::env::current_dir().context("no current directory")?);
    let origin = git.run(&["config", "remote.origin.url"])?;
    let repo = Repo::of_origin(&origin)
        .with_context(|| format!("origin {origin} is not a GitHub repository"))?;
    Ok((git, origin, repo))
}

/// The repository a run with no Issue URL started from the Launch directory
/// is on.
pub fn repo() -> Result<Repo> {
    directory().map(|(_, _, repo)| repo)
}

/// Try, without waiting, for the lock that makes its holder the one Architect
/// run or Pickup run on `repo` on this machine: an advisory lock on a file
/// under `~/.thirdshift` named for the repository, as GitHub compares names,
/// whatever their case. It is held until the file returned is closed, or the
/// process ends. `None` if another process holds it: one of them on `repo` is
/// still running. The lock file itself is never deleted, and means nothing
/// unless locked. Its directory keeps the name it had when only an Architect
/// run took the lock, so that a thirdshift from before the Pickup run and
/// one from after still exclude each other.
fn try_run_lock(repo: &Repo) -> Result<Option<File>> {
    let dir = config::home()?
        .join(".thirdshift/architect-locks")
        .join(repo.owner.to_ascii_lowercase());
    fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
    let path = dir.join(format!("{}.lock", repo.name.to_ascii_lowercase()));
    let file = File::create(&path).with_context(|| format!("can't open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("can't lock {}", path.display()))
        }
    }
}
