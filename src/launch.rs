//! The Launch directory, as every command opens it: a Run, from an Issue
//! URL, or a run with no Issue URL, an Architect run or a Pickup run. Its
//! `origin` names the repository, git has the identity an agent commits
//! with, and a branch is checked out there, or none on a detached HEAD. The
//! Base branch is the one named for the run, or else the one checked out
//! there, and must be on origin with no local copy ahead of it. With
//! `launch.pull`, a checked-out Base branch is fast-forwarded to the origin
//! commit sampled by its preparation.
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
use crate::github::GitHub;
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

/// A validated Base branch bound to its Launch directory. Validation and
/// its optional checkout update share one origin commit sampled at
/// preparation; later Worktree acquisition fetches independently.
pub struct BaseBranch {
    git: Git,
    name: String,
    origin_commit: String,
    checked_out: bool,
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
    /// [`LaunchDirectory::prepare_base_branch`], once Issue branch selection
    /// has said what names it.
    pub fn open_for_run(issue: &IssueUrl) -> Result<Self> {
        let directory = Self::open(current_dir()?, Opening::Run)?;
        if !issue.matches_origin(&directory.origin) {
            bail!(
                "origin mismatch: {} is not in the repository at origin {}",
                issue.url,
                directory.origin
            );
        }
        if !GitHub::new().issue_is_open(issue)? {
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
            .run_optional(&["symbolic-ref", "--quiet", "HEAD"])?
            .and_then(|reference| reference.strip_prefix("refs/heads/").map(String::from));
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
    /// origin lacks. The returned value retains that sampled origin commit
    /// for its optional checkout update, without changing the checkout now.
    pub fn prepare_base_branch(&self, named: Option<&str>) -> Result<BaseBranch> {
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
        self.git.fetch(&[base])?;
        let origin_commit = self.git.run(&[
            "rev-parse",
            "--verify",
            &format!("refs/remotes/origin/{base}^{{commit}}"),
        ])?;
        let local_ref = format!("refs/heads/{base}");
        if let Some(local_commit) = self.git.run_optional(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{local_ref}^{{commit}}"),
        ])? {
            let ahead = self.git.run(&[
                "rev-list",
                "--count",
                &format!("{origin_commit}..{local_commit}"),
            ])?;
            if ahead != "0" {
                bail!("local {base} is {ahead} commit(s) ahead of origin/{base}; push them first");
            }
        }
        Ok(BaseBranch {
            git: Git::new(self.git.dir()),
            name: base.to_string(),
            origin_commit,
            checked_out: self
                .git
                .run_optional(&["symbolic-ref", "--quiet", "HEAD"])?
                .as_deref()
                == Some(local_ref.as_str()),
        })
    }
}

impl BaseBranch {
    /// The validated Base branch's name, for prompts and later acquisition.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The `launch.pull` fast-forward toward the sampled origin commit,
    /// only while the Base branch is still checked out as at preparation.
    /// No run depends on this, so a failure is only a warning and local
    /// work is preserved.
    pub fn pull(&self) {
        if !self.checked_out {
            return;
        }
        let base = self.name();
        let origin_base = format!("origin/{base}");
        match self.needs_update() {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                progress::warn(
                    &error,
                    format_args!("could not check whether {base} is up to date"),
                );
                return;
            }
        }
        progress::step(format_args!(
            "updating {base} in the Launch directory from {origin_base}"
        ));
        // Preparation precedes review ownership inspection. A fast-forward
        // must not let Git's maintenance prune retained registrations either.
        if let Err(error) = self.git.run(&[
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
            "merge",
            "--ff-only",
            "--quiet",
            &self.origin_commit,
        ]) {
            progress::warn(
                &error,
                format_args!(
                    "could not update {base} in the Launch directory, \
                     so update it by hand: git pull --ff-only origin {base}"
                ),
            );
        }
    }

    /// A changed checkout is ineligible; a checkout already containing the
    /// sampled origin commit needs no update.
    fn needs_update(&self) -> Result<bool> {
        if self
            .git
            .run_optional(&["symbolic-ref", "--quiet", "HEAD"])?
            != Some(format!("refs/heads/{}", self.name))
        {
            return Ok(false);
        }
        Ok(!self
            .git
            .succeeds(&["merge-base", "--is-ancestor", &self.origin_commit, "HEAD"])?)
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
    pub base: BaseBranch,
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
    let base = directory.prepare_base_branch(base)?;
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

        let base = opened(&work, Opening::Run)
            .prepare_base_branch(None)
            .unwrap();

        assert_eq!(base.name(), "main");
    }

    #[test]
    fn a_named_branch_wins_over_the_checked_out_one() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["push", "-q", "origin", "main:release"]);
        isolated(&work, &["checkout", "-q", "-b", "topic"]);

        let base = opened(&work, Opening::Run)
            .prepare_base_branch(Some("release"))
            .unwrap();

        assert_eq!(base.name(), "release");
    }

    #[test]
    fn a_named_branch_settles_the_base_branch_on_a_detached_head() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, Some(("file.txt", "origin's\n")));
        isolated(&work, &["checkout", "-q", "--detach"]);
        let before = head(&work);
        let directory = opened(&work, Opening::Pass);
        let base = directory.prepare_base_branch(Some("main")).unwrap();

        assert_eq!(directory.checked_out(), None);
        assert_eq!(base.name(), "main");
        base.pull();
        assert_eq!(head(&work), before);
        assert!(!work.join("file.txt").exists());
        isolated(&work, &["checkout", "-q", "main"]);
        base.pull();
        assert_eq!(head(&work), before);
        assert!(!work.join("file.txt").exists());
    }

    #[test]
    fn a_detached_head_with_no_named_branch_fails_a_run_without_the_base_hint() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "--detach"]);

        let error = opened(&work, Opening::Run)
            .prepare_base_branch(None)
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "HEAD is detached; check out the branch the work should be based on"
        );
    }

    #[test]
    fn a_detached_head_with_no_named_branch_fails_a_pass_with_the_base_hint() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["checkout", "-q", "--detach"]);

        let error = opened(&work, Opening::Pass)
            .prepare_base_branch(None)
            .err()
            .unwrap();

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

        let error = opened(&work, Opening::Run)
            .prepare_base_branch(None)
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "base branch unpushed does not exist on origin; push it first"
        );
    }

    #[test]
    fn a_local_base_branch_ahead_of_origin_fails() {
        let (_temp, work) = launch_with_origin();
        isolated(&work, &["commit", "-q", "--allow-empty", "-m", "Local"]);

        let error = opened(&work, Opening::Pass)
            .prepare_base_branch(None)
            .err()
            .unwrap();

        assert_eq!(
            error.to_string(),
            "local main is 1 commit(s) ahead of origin/main; push them first"
        );
    }

    #[test]
    fn shadow_refs_cannot_hide_unpushed_base_branch_work() {
        for shadow in [
            "refs/tags/main",
            "refs/tags/origin/main",
            "refs/heads/origin/main",
        ] {
            let (_temp, work) = launch_with_origin();
            let origin = head(&work);
            isolated(&work, &["commit", "-q", "--allow-empty", "-m", "Local"]);
            let local = head(&work);
            let shadow_commit = if shadow == "refs/tags/main" {
                &origin
            } else {
                &local
            };
            isolated(&work, &["update-ref", shadow, shadow_commit]);
            let refs = isolated(&work, &["show-ref"]);

            let error = opened(&work, Opening::Run)
                .prepare_base_branch(None)
                .err()
                .expect("unpushed Base branch work must be rejected");

            assert_eq!(
                error.to_string(),
                "local main is 1 commit(s) ahead of origin/main; push them first",
                "{shadow}"
            );
            assert_eq!(isolated(&work, &["show-ref"]), refs, "{shadow}");
            assert_eq!(head(&work), local, "{shadow}");
        }
    }

    #[test]
    fn the_pull_fast_forwards_the_checked_out_base_branch() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, None);
        let directory = opened(&work, Opening::Run);
        let base = directory.prepare_base_branch(None).unwrap();

        base.pull();

        assert_eq!(head(&work), isolated(&work, &["rev-parse", "origin/main"]));
    }

    #[test]
    fn tracking_ref_changes_cannot_change_the_prepared_pull_target() {
        for mutation in ["advance", "rewind", "replace", "delete"] {
            let (_temp, work) = launch_with_origin();
            let before = head(&work);
            advance_origin(&work, Some(("file.txt", "sampled origin\n")));
            let expected = head(&work.parent().unwrap().join("other"));
            let base = opened(&work, Opening::Run)
                .prepare_base_branch(None)
                .unwrap();
            let tracking = "refs/remotes/origin/main";
            match mutation {
                "advance" => {
                    let tree = isolated(&work, &["rev-parse", &format!("{expected}^{{tree}}")]);
                    let later = isolated(
                        &work,
                        &["commit-tree", &tree, "-p", &expected, "-m", "Later origin"],
                    );
                    isolated(&work, &["update-ref", tracking, &later]);
                }
                "rewind" => {
                    isolated(&work, &["update-ref", tracking, &before]);
                }
                "replace" => {
                    let unrelated = unrelated_commit(&work);
                    isolated(&work, &["update-ref", tracking, &unrelated]);
                }
                "delete" => {
                    isolated(&work, &["update-ref", "-d", tracking]);
                }
                _ => unreachable!(),
            }
            let tracking_after = isolated(
                &work,
                &[
                    "for-each-ref",
                    "--format=%(refname) %(objectname)",
                    tracking,
                ],
            );

            base.pull();

            assert_eq!(head(&work), expected, "{mutation}");
            assert_eq!(
                std::fs::read_to_string(work.join("file.txt")).unwrap(),
                "sampled origin\n",
                "{mutation}"
            );
            assert_eq!(
                isolated(
                    &work,
                    &[
                        "for-each-ref",
                        "--format=%(refname) %(objectname)",
                        tracking
                    ]
                ),
                tracking_after,
                "{mutation}"
            );
        }
    }

    /// An unrelated root commit, available locally without moving any ref.
    fn unrelated_commit(work: &Path) -> String {
        let tree = isolated(work, &["rev-parse", "HEAD^{tree}"]);
        isolated(work, &["commit-tree", &tree, "-m", "Unrelated history"])
    }

    #[test]
    fn equal_or_behind_base_branches_ignore_shadow_refs_and_preserve_them_on_pull() {
        for behind in [false, true] {
            for shadows in [
                &["refs/tags/main"][..],
                &["refs/tags/origin/main"][..],
                &["refs/heads/origin/main"][..],
                &[
                    "refs/tags/main",
                    "refs/tags/origin/main",
                    "refs/heads/origin/main",
                ][..],
            ] {
                let (_temp, work) = launch_with_origin();
                let before = head(&work);
                let expected = if behind {
                    advance_origin(&work, Some(("file.txt", "origin's\n")));
                    head(&work.parent().unwrap().join("other"))
                } else {
                    before.clone()
                };
                let unrelated = unrelated_commit(&work);
                for shadow in shadows {
                    isolated(&work, &["update-ref", shadow, &unrelated]);
                }

                let base = opened(&work, Opening::Run)
                    .prepare_base_branch(None)
                    .unwrap();

                assert_eq!(base.name(), "main");
                assert_eq!(head(&work), before, "preparation changed the checkout");
                base.pull();
                assert_eq!(head(&work), expected, "behind: {behind}, {shadows:?}");
                for shadow in shadows {
                    assert_eq!(
                        isolated(&work, &["rev-parse", "--verify", shadow]),
                        unrelated,
                        "{shadow}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_pull_leaves_a_base_branch_that_is_not_checked_out_alone() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, None);
        isolated(&work, &["checkout", "-q", "-b", "topic"]);
        let before = head(&work);
        let directory = opened(&work, Opening::Pass);
        let base = directory.prepare_base_branch(Some("main")).unwrap();

        base.pull();

        assert_eq!(head(&work), before);
        assert_eq!(isolated(&work, &["rev-parse", "main"]), before);
        isolated(&work, &["checkout", "-q", "main"]);
        base.pull();
        assert_eq!(head(&work), before);
    }

    #[test]
    fn a_deferred_pull_leaves_a_switched_or_detached_checkout_alone() {
        for checkout in [
            &["checkout", "-q", "-b", "topic"][..],
            &["checkout", "-q", "--detach"][..],
        ] {
            let (_temp, work) = launch_with_origin();
            advance_origin(&work, Some(("file.txt", "origin's\n")));
            let base = opened(&work, Opening::Pass)
                .prepare_base_branch(None)
                .unwrap();
            isolated(&work, checkout);
            let before = head(&work);
            let refs = isolated(&work, &["show-ref"]);

            base.pull();

            assert_eq!(head(&work), before, "{checkout:?}");
            assert_eq!(isolated(&work, &["show-ref"]), refs, "{checkout:?}");
            assert!(!work.join("file.txt").exists(), "{checkout:?}");
        }
    }

    #[test]
    fn a_pull_that_cant_fast_forward_leaves_the_checkout_alone() {
        let (_temp, work) = launch_with_origin();
        advance_origin(&work, Some(("file.txt", "origin's\n")));
        std::fs::write(work.join("file.txt"), "uncommitted\n").unwrap();
        let before = head(&work);
        let directory = opened(&work, Opening::Run);
        let base = directory.prepare_base_branch(None).unwrap();

        base.pull();

        assert_eq!(head(&work), before);
        assert_eq!(
            std::fs::read_to_string(work.join("file.txt")).unwrap(),
            "uncommitted\n"
        );
    }

    #[test]
    fn preparation_and_dropping_the_base_branch_leave_local_work_untouched() {
        let (_temp, work) = launch_with_origin();
        std::fs::write(work.join("file.txt"), "original\n").unwrap();
        isolated(&work, &["add", "file.txt"]);
        isolated(&work, &["commit", "-q", "-m", "Tracked file"]);
        isolated(&work, &["push", "-q", "origin", "main"]);
        advance_origin(&work, Some(("file.txt", "origin's\n")));
        std::fs::write(work.join("file.txt"), "staged\n").unwrap();
        isolated(&work, &["add", "file.txt"]);
        std::fs::write(work.join("file.txt"), "unstaged\n").unwrap();
        let branches = isolated(&work, &["for-each-ref", "refs/heads/"]);
        let status = isolated(&work, &["status", "--porcelain"]);
        let index = std::fs::read(work.join(".git/index")).unwrap();

        let base = opened(&work, Opening::Run)
            .prepare_base_branch(None)
            .unwrap();
        assert_eq!(base.name(), "main");
        drop(base);

        assert_eq!(isolated(&work, &["for-each-ref", "refs/heads/"]), branches);
        assert_eq!(std::fs::read(work.join(".git/index")).unwrap(), index);
        assert_eq!(
            std::fs::read_to_string(work.join("file.txt")).unwrap(),
            "unstaged\n"
        );
        assert_eq!(isolated(&work, &["status", "--porcelain"]), status);
    }

    #[test]
    fn a_deferred_pull_preserves_commits_made_after_preparation() {
        for contains_origin in [false, true] {
            let (_temp, work) = launch_with_origin();
            advance_origin(&work, Some(("file.txt", "origin's\n")));
            let expected = head(&work.parent().unwrap().join("other"));
            let base = opened(&work, Opening::Run)
                .prepare_base_branch(None)
                .unwrap();
            if contains_origin {
                isolated(&work, &["merge", "--ff-only", &expected]);
            }
            std::fs::write(work.join("local.txt"), "local work\n").unwrap();
            isolated(&work, &["add", "local.txt"]);
            isolated(
                &work,
                &["commit", "-q", "-m", "Local work after preparation"],
            );
            let before = head(&work);
            let refs = isolated(&work, &["show-ref"]);
            let index = std::fs::read(work.join(".git/index")).unwrap();

            base.pull();

            assert_eq!(head(&work), before, "contains origin: {contains_origin}");
            assert_eq!(isolated(&work, &["show-ref"]), refs);
            assert_eq!(std::fs::read(work.join(".git/index")).unwrap(), index);
            assert_eq!(
                std::fs::read_to_string(work.join("local.txt")).unwrap(),
                "local work\n"
            );
            assert_eq!(work.join("file.txt").exists(), contains_origin);
        }
    }

    #[test]
    fn a_prepared_pull_preserves_recorded_command_interruption() {
        crate::test_support::with_recorded_signal(
            "launch::tests::a_prepared_pull_preserves_recorded_command_interruption",
            |signal| {
                let (_temp, work) = launch_with_origin();
                advance_origin(&work, Some(("file.txt", "origin's\n")));
                let directory = opened(&work, Opening::Run);
                let base = directory.prepare_base_branch(None).unwrap();
                let before = head(&work);
                let refs = isolated(&work, &["show-ref"]);
                signal_hook::low_level::raise(signal).unwrap();

                base.pull();

                assert!(crate::interrupt::requested());
                assert_eq!(head(&work), before);
                assert_eq!(isolated(&work, &["show-ref"]), refs);
                assert!(!work.join("file.txt").exists());
                assert_eq!(
                    directory
                        .prepare_base_branch(None)
                        .err()
                        .unwrap()
                        .to_string(),
                    "interrupted"
                );
            },
        );
    }
}
