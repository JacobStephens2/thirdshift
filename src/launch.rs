//! The Launch directory, as every command opens it: a Run, from an Issue
//! URL, or a run with no Issue URL, an Architect run or a Pickup run. Its
//! `origin` names the repository, git has the identity an agent commits
//! with, and a branch is checked out there, or none on a detached HEAD. The
//! Base branch is the one named for the run, or else the one checked out
//! there, and must be on origin with no local copy ahead of it. With
//! `launch.pull`, a checked-out Base branch is fast-forwarded to origin's.
//!
//! A Run's opening first makes the Origin match and checks that its issue is
//! open. A run with no Issue URL instead fails on an `origin` that isn't on
//! GitHub, settles its Base branch at once, then tries a lock, so only one
//! of them per repository runs at a time on a machine.

use std::fmt;
use std::fs::{self, File, TryLockError};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::config;
use crate::git::Git;
use crate::github;
use crate::issue::{IssueUrl, Repo};
use crate::progress;

/// The Launch directory, opened by a Run or by a run with no Issue URL, and
/// the facts about it its opening read.
pub struct LaunchDirectory {
    git: Git,
    /// The URL of its `origin`.
    origin: String,
    /// The branch checked out there, or none on a detached HEAD.
    checked_out: Option<String>,
    opening: Opening,
}

/// Which command opened the Launch directory, which says how a detached HEAD
/// with no Base branch named fails it.
#[derive(Clone, Copy)]
enum Opening {
    /// A Run, from an Issue URL.
    Run,
    /// A run with no Issue URL, an Architect run or a Pickup run, whose
    /// command can name its Base branch with `base <branch>`.
    Pass,
}

impl LaunchDirectory {
    /// Open the Launch directory, the current directory, for a Run on
    /// `issue`: `origin` must name `issue`'s repository, the Origin match,
    /// `issue` must be open, and git must have the identity an agent commits
    /// with, checked in that order. Its Base branch is settled later, by
    /// [`LaunchDirectory::base_branch`], once Issue branch selection has said
    /// what names it.
    pub fn open_for_run(issue: &IssueUrl) -> Result<Self> {
        let directory = Self::open(current_dir()?, Opening::Run)?;
        if !issue.matches_origin(&directory.origin) {
            bail!(
                "origin mismatch: {} is not in the repository at origin {}",
                issue.url,
                directory.origin
            );
        }
        if !github::issue_is_open(issue)? {
            bail!("issue #{} is closed", issue.number);
        }
        directory.check_identity()?;
        Ok(directory)
    }

    /// The Launch directory `dir`, opened by `opening`, with its `origin`
    /// and the branch checked out there read, but nothing checked.
    fn open(dir: PathBuf, opening: Opening) -> Result<Self> {
        let git = Git::new(dir);
        let origin = git.run(&["config", "remote.origin.url"])?;
        let checked_out = git
            .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok();
        Ok(LaunchDirectory {
            git,
            origin,
            checked_out,
            opening,
        })
    }

    /// `git` in the Launch directory.
    pub fn git(&self) -> &Git {
        &self.git
    }

    /// The URL of its `origin`.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The branch checked out there, or none on a detached HEAD.
    pub fn checked_out(&self) -> Option<&str> {
        self.checked_out.as_deref()
    }

    /// The GitHub repository `origin` names.
    fn repo(&self) -> Result<Repo> {
        Repo::of_origin(&self.origin)
            .with_context(|| format!("origin {} is not a GitHub repository", self.origin))
    }

    /// Check that git has the name and email an agent commits with.
    fn check_identity(&self) -> Result<()> {
        for (key, example_value) in [
            ("user.name", r#""Your Name""#),
            ("user.email", "you@example.com"),
        ] {
            if self.git.run(&["config", "--default", "", key])?.is_empty() {
                bail!(
                    "git {key} is not set; the agent needs it to commit. \
                     Set it with: git config --global {key} {example_value}"
                );
            }
        }
        Ok(())
    }

    /// The Base branch: `named`, the branch named for the run, whatever is
    /// checked out here, or without one the branch checked out, so a
    /// detached HEAD with neither fails. It must exist on origin, which is
    /// fetched, and the local copy of it, if any, must have no commits
    /// origin lacks.
    pub fn base_branch(&self, named: Option<&str>) -> Result<String> {
        let Some(base) = named.or(self.checked_out()) else {
            match self.opening {
                Opening::Run => {
                    bail!("HEAD is detached; check out the branch the work should be based on")
                }
                Opening::Pass => bail!(
                    "HEAD is detached; check out the branch the work should be based on, \
                     or name it with base <branch>"
                ),
            }
        };
        if !self.git.on_origin(base)? {
            bail!("base branch {base} does not exist on origin; push it first");
        }
        self.git.run(&["fetch", "origin", base])?;
        let local = format!("refs/heads/{base}");
        if self
            .git
            .run(&["rev-parse", "--verify", "--quiet", &local])
            .is_ok()
        {
            let ahead =
                self.git
                    .run(&["rev-list", "--count", &format!("origin/{base}..{base}")])?;
            if ahead != "0" {
                bail!("local {base} is {ahead} commit(s) ahead of origin/{base}; push them first");
            }
        }
        Ok(base.to_string())
    }

    /// The `launch.pull` fast-forward: bring the Base branch `base` up to
    /// date with `origin/<base>`, if it is the branch checked out here. Call
    /// it after [`LaunchDirectory::base_branch`], which fetches
    /// `origin/<base>` and fails if `base` is ahead of it. No run depends on
    /// this, so a failure, such as uncommitted changes in the way, is only a
    /// warning, and those changes are left as they were.
    pub fn pull(&self, base: &str) {
        if self.checked_out() != Some(base) {
            return;
        }
        let origin_base = format!("origin/{base}");
        let up_to_date = self
            .git
            .succeeds(&["merge-base", "--is-ancestor", &origin_base, "HEAD"]);
        if up_to_date.unwrap_or(false) {
            return;
        }
        progress::step(format_args!(
            "updating {base} in the Launch directory from {origin_base}"
        ));
        if let Err(error) = self
            .git
            .run(&["merge", "--ff-only", "--quiet", &origin_base])
        {
            progress::warn(
                &error,
                format_args!(
                    "could not update {base} in the Launch directory, \
                     so update it by hand: git pull --ff-only origin {base}"
                ),
            );
        }
    }
}

/// The Launch directory, the current directory.
fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("no current directory")
}

/// The Launch directory of a run with no Issue URL, its checks passed and
/// its repository's lock held.
pub struct Launch {
    pub directory: LaunchDirectory,
    /// The GitHub repository `origin` names.
    pub repo: Repo,
    /// The run's Base branch.
    pub base: String,
}

/// How a run with no Issue URL starts.
pub enum Start {
    /// Its checks passed and its repository's lock is its own.
    Clear(Launch),
    /// It is skipped, with nothing done.
    AlreadyRunning(AlreadyRunning),
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
/// checked out there, so a detached HEAD with neither fails it.
///
/// Once the preflight checks pass, the run is [`AlreadyRunning`], with
/// nothing done, if another on the same repository is still running on this
/// machine. Otherwise this process is that repository's one such run until it
/// exits, through whatever it dispatches.
pub fn start(base: Option<&str>) -> Result<Start> {
    let directory = LaunchDirectory::open(current_dir()?, Opening::Pass)?;
    let repo = directory.repo()?;
    directory.check_identity()?;
    let base = directory.base_branch(base)?;
    let Some(lock) = try_run_lock(&repo)? else {
        return Ok(Start::AlreadyRunning(AlreadyRunning(repo)));
    };
    // Never closed, so the lock is held for as long as this process lives,
    // through the Spec run or Run it dispatches, and the operating system
    // releases it however the process ends.
    std::mem::forget(lock);
    Ok(Start::Clear(Launch {
        directory,
        repo,
        base,
    }))
}

/// The repository a run with no Issue URL started from the Launch directory
/// is on.
pub fn repo() -> Result<Repo> {
    LaunchDirectory::open(current_dir()?, Opening::Pass)?.repo()
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;

    /// A repository in a temp directory, with an identity and one commit on
    /// `main`, pushed to a bare repository beside it, which its `origin`
    /// names as the GitHub repository acme/widgets. No global or system
    /// config is read.
    fn launch_with_origin() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::TempDir::new().unwrap();
        isolated(temp.path(), &["init", "-q", "--bare", "origin.git"]);
        isolated(temp.path(), &["init", "-q", "-b", "main", "work"]);
        let work = temp.path().join("work");
        let bare = temp.path().join("origin.git");
        for (key, value) in [
            ("user.name", "Test Runner"),
            ("user.email", "runner@example.com"),
            ("remote.origin.url", "https://github.com/acme/widgets.git"),
            ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
            (
                &*format!("url.{}.insteadOf", bare.display()),
                "https://github.com/acme/widgets.git",
            ),
        ] {
            isolated(&work, &["config", key, value]);
        }
        isolated(&work, &["commit", "-q", "--allow-empty", "-m", "Initial"]);
        isolated(&work, &["push", "-q", "origin", "main"]);
        (temp, work)
    }

    fn isolated(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn opened(work: &Path, opening: Opening) -> LaunchDirectory {
        LaunchDirectory::open(work.to_path_buf(), opening).unwrap()
    }

    /// Commit on `main` in a clone of origin beside `work`, writing `file`
    /// with `contents` if given, and push it, so origin's `main` is a commit
    /// ahead of `work`'s.
    fn advance_origin(work: &Path, file: Option<(&str, &str)>) {
        let temp = work.parent().unwrap();
        isolated(temp, &["clone", "-q", "-b", "main", "origin.git", "other"]);
        let other = temp.join("other");
        if let Some((name, contents)) = file {
            std::fs::write(other.join(name), contents).unwrap();
            isolated(&other, &["add", name]);
        }
        isolated(
            &other,
            &[
                "-c",
                "user.name=Other",
                "-c",
                "user.email=other@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "Later",
            ],
        );
        isolated(&other, &["push", "-q", "origin", "main"]);
    }

    fn head(work: &Path) -> String {
        isolated(work, &["rev-parse", "HEAD"])
    }

    #[test]
    fn the_opening_reads_origin_and_the_checked_out_branch() {
        let (_temp, work) = launch_with_origin();

        let directory = opened(&work, Opening::Pass);

        assert_eq!(directory.origin(), "https://github.com/acme/widgets.git");
        assert_eq!(directory.checked_out(), Some("main"));
        assert_eq!(
            directory.repo().unwrap(),
            Repo {
                owner: "acme".into(),
                name: "widgets".into()
            }
        );
    }

    #[test]
    fn the_checked_out_branch_is_the_base_branch_when_none_is_named() {
        let (_temp, work) = launch_with_origin();

        let base = opened(&work, Opening::Run).base_branch(None).unwrap();

        assert_eq!(base, "main");
    }

    #[test]
    fn a_named_branch_wins_over_the_checked_out_one() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["push", "-q", "origin", "main:release"]);
        isolated(&work, &["checkout", "-q", "-b", "topic"]);

        let base = opened(&work, Opening::Run)
            .base_branch(Some("release"))
            .unwrap();

        assert_eq!(base, "release");
    }

    #[test]
    fn a_named_branch_settles_the_base_branch_on_a_detached_head() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "--detach"]);

        let directory = opened(&work, Opening::Pass);

        assert_eq!(directory.checked_out(), None);
        assert_eq!(directory.base_branch(Some("main")).unwrap(), "main");
    }

    #[test]
    fn a_detached_head_with_no_named_branch_fails_a_run_without_the_base_hint() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "--detach"]);

        let error = opened(&work, Opening::Run).base_branch(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            "HEAD is detached; check out the branch the work should be based on"
        );
    }

    #[test]
    fn a_detached_head_with_no_named_branch_fails_a_pass_with_the_base_hint() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "--detach"]);

        let error = opened(&work, Opening::Pass).base_branch(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            "HEAD is detached; check out the branch the work should be based on, \
             or name it with base <branch>"
        );
    }

    #[test]
    fn a_base_branch_missing_on_origin_fails() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "-b", "unpushed"]);

        let error = opened(&work, Opening::Run).base_branch(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            "base branch unpushed does not exist on origin; push it first"
        );
    }

    #[test]
    fn a_local_base_branch_ahead_of_origin_fails() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["commit", "-q", "--allow-empty", "-m", "Local"]);

        let error = opened(&work, Opening::Pass).base_branch(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            "local main is 1 commit(s) ahead of origin/main; push them first"
        );
    }

    #[test]
    fn the_pull_fast_forwards_the_checked_out_base_branch() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, None);
        let directory = opened(&work, Opening::Run);
        let base = directory.base_branch(None).unwrap();

        directory.pull(&base);

        assert_eq!(head(&work), isolated(&work, &["rev-parse", "origin/main"]));
    }

    #[test]
    fn the_pull_leaves_a_base_branch_that_is_not_checked_out_alone() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, None);
        isolated(&work, &["checkout", "-q", "-b", "topic"]);
        let before = head(&work);
        let directory = opened(&work, Opening::Pass);
        let base = directory.base_branch(Some("main")).unwrap();

        directory.pull(&base);

        assert_eq!(head(&work), before);
        assert_eq!(isolated(&work, &["rev-parse", "main"]), before);
    }

    #[test]
    fn a_pull_that_cant_fast_forward_leaves_the_checkout_alone() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, Some(("file.txt", "origin's\n")));
        std::fs::write(work.join("file.txt"), "uncommitted\n").unwrap();
        let before = head(&work);
        let directory = opened(&work, Opening::Run);
        let base = directory.base_branch(None).unwrap();

        directory.pull(&base);

        assert_eq!(head(&work), before);
        assert_eq!(
            std::fs::read_to_string(work.join("file.txt")).unwrap(),
            "uncommitted\n"
        );
    }
}
