//! Running `git` in a directory.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::process::{self, Control, Interruption};
use crate::progress;

#[cfg(test)]
mod execution_tests;

/// `git` bound to a working directory. Output is captured, never passed
/// through, so stdout stays reserved for the PR URL.
pub struct Git {
    dir: PathBuf,
    interruption: Interruption,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Git {
            dir: dir.into(),
            interruption: Interruption::Ordinary,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// An independent view for finishing work in the same directory. It
    /// ignores recorded interruption without changing this or other views.
    pub fn completion(&self) -> Self {
        Git {
            dir: self.dir.clone(),
            interruption: Interruption::Completion,
        }
    }

    /// Run `git <args>` and return its trimmed stdout. If it exits non-zero,
    /// fail with git's own `error:` or `fatal:` line, if it wrote one, then
    /// the last lines of its stderr and of its stdout, where a hook's
    /// explanation can end up. While it fails on a lock file another git
    /// holds, or on a ref another git moved meanwhile, as when a Spec run's
    /// Tickets fetch or create worktrees from one Launch directory at once,
    /// it is run again, for up to [`LOCK_WAIT`].
    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_checked(args, || Ok(()))
    }

    /// Keep a caller's authority check around every subprocess, including
    /// retries. A completed helper cannot authorize its next command attempt.
    pub(crate) fn run_checked(
        &self,
        args: &[&str],
        check: impl Fn() -> Result<()>,
    ) -> Result<String> {
        let output = self.retried_output_checked(args, &check)?;
        if !output.status.success() {
            let tail = [error_first(&output.stderr), last_lines(&output.stdout)].concat();
            let mut message = format!("git {} failed", args.join(" "));
            if !tail.is_empty() {
                message = format!("{message}: {}", tail.join("\n"));
            }
            bail!(message);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// A probe's trimmed stdout, or no answer for a completed nonzero exit.
    /// Interruption and transport failures remain errors, never absence.
    pub fn run_optional(&self, args: &[&str]) -> Result<Option<String>> {
        let output = self.retried_output(args)?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string()))
    }

    fn retried_output(&self, args: &[&str]) -> Result<Output> {
        self.retried_output_checked(args, &|| Ok(()))
    }

    fn retried_output_checked(
        &self,
        args: &[&str],
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Output> {
        let attempt = || {
            check()?;
            let result = self.output(args);
            check()?;
            result
        };
        let deadline = Instant::now() + LOCK_WAIT;
        let mut output = attempt()?;
        while !output.status.success() && held_lock(&output.stderr) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            output = attempt()?;
        }
        Ok(output)
    }

    /// The repository's common git directory, the one its worktrees share.
    pub fn common_dir(&self) -> Result<PathBuf> {
        Ok(self.dir.join(self.run(&["rev-parse", "--git-common-dir"])?))
    }

    /// Wait for, then hold until the file returned is dropped, a lock on the
    /// file `name` in the repository's common git directory, which every Run
    /// from one Launch directory shares. Ordinary acquisition stops for a
    /// recorded interruption; completion acquisition keeps waiting. Contention
    /// is polled every 100 ms without a deadline, and the file is never removed.
    pub fn lock(&self, name: &str) -> Result<File> {
        self.lock_checked(name, &|| Ok(()))
    }

    fn lock_checked(&self, name: &str, check: &dyn Fn() -> Result<()>) -> Result<File> {
        let common = self
            .dir
            .join(self.run_checked(&["rev-parse", "--git-common-dir"], check)?);
        let path = common.join(name);
        check()?;
        self.interruption.check()?;
        let file = File::create(&path).with_context(|| format!("can't open {}", path.display()))?;
        loop {
            self.interruption.check()?;
            check()?;
            match file.try_lock() {
                Ok(()) => {
                    self.interruption.check()?;
                    return Ok(file);
                }
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("can't lock {}", path.display()));
                }
            }
        }
    }

    /// Keep fetch's local connectivity check away from incomplete or
    /// disappearing Worktree heads. Acquisition and disposal take this
    /// after the worktree ownership lock; fetch takes only this lock.
    pub fn lock_worktree_refs(&self) -> Result<File> {
        self.lock("thirdshift-worktree-refs.lock")
    }

    /// Whether origin has the branch `branch`, as origin itself answers.
    pub fn on_origin(&self, branch: &str) -> Result<bool> {
        self.on_origin_checked(branch, || Ok(()))
    }

    pub(crate) fn on_origin_checked(
        &self,
        branch: &str,
        check: impl Fn() -> Result<()>,
    ) -> Result<bool> {
        let reference = format!("refs/heads/{branch}");
        let found = self.run_checked(&["ls-remote", "--heads", "origin", &reference], check)?;
        Ok(!found.is_empty())
    }

    /// Fetch origin without automatic maintenance: worktree disposal must
    /// pass ownership inspection, never a fetch's implicit pruning.
    pub fn fetch(&self, branches: &[&str]) -> Result<()> {
        self.fetch_checked(branches, || Ok(()))
    }

    /// An acquired operation rechecks ownership after waiting for refs and
    /// before the fetch can change them. The refs lock remains local to fetch.
    pub(crate) fn fetch_checked(
        &self,
        branches: &[&str],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let _lock = self.lock_checked("thirdshift-worktree-refs.lock", &check)?;
        let mut args = vec!["fetch", "--no-auto-maintenance", "origin"];
        args.extend_from_slice(branches);
        self.run_checked(&args, check)?;
        Ok(())
    }

    /// Push `branch` to origin without local hooks: sessions and CI check
    /// the work, and a rejecting hook must not strand Failed run salvage.
    pub fn push(&self, branch: &str) -> Result<()> {
        self.push_checked(branch, || Ok(()))
    }

    pub(crate) fn push_checked(&self, branch: &str, check: impl Fn() -> Result<()>) -> Result<()> {
        progress::step(format_args!("pushing {branch}"));
        self.run_checked(&["push", "--no-verify", "origin", branch], check)?;
        Ok(())
    }

    /// Whether a merge is in progress in this directory.
    pub fn merge_in_progress(&self) -> Result<bool> {
        self.succeeds(&["rev-parse", "-q", "--verify", "MERGE_HEAD"])
    }

    /// Run `git <args>` and report whether it exited zero, for commands whose
    /// exit status is the answer. Only a completed nonzero exit is false;
    /// interruption and transport failures remain errors.
    pub fn succeeds(&self, args: &[&str]) -> Result<bool> {
        Ok(self.output(args)?.status.success())
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        process::output(
            Command::new("git").args(args).current_dir(&self.dir),
            None,
            Control {
                name: &format!("git {}", args.join(" ")),
                interruption: self.interruption,
                stop: &|child| process::stop(child, &[libc::SIGTERM]),
            },
        )
    }
}

/// How long [`Git::run`] keeps trying a command that fails on a lock file
/// or on a ref another git moved meanwhile.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// Whether git's `stderr` says it failed because a lock file, such as
/// `config.lock` or a ref's, already exists, or because another git moved a
/// ref between reading and updating it, as two fetches of one branch can,
/// in older git's wording or git 2.52's.
fn held_lock(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr);
    stderr.contains(".lock': File exists")
        || stderr.contains("could not lock config file")
        || (stderr.contains("cannot lock ref") && stderr.contains("but expected"))
        || stderr.contains("incorrect old value provided")
}

/// The most lines of each stream a failure reports.
const MAX_LINES: usize = 10;

/// The last `MAX_LINES` non-empty lines of `stream`, trimmed.
fn last_lines(stream: &[u8]) -> Vec<String> {
    let mut lines = non_empty_lines(stream);
    lines.drain(..lines.len().saturating_sub(MAX_LINES));
    lines
}

/// The lines of a failed git's `stderr` to report, `MAX_LINES` at most: the
/// first line starting `error:` or `fatal:`, which says what went wrong,
/// then the last of the others. With no such line, its last lines. So a
/// reader of the first line alone, as of a child Run's cause, reads git's
/// error and not a banner such as a fetch's `From <url>`.
fn error_first(stderr: &[u8]) -> Vec<String> {
    let mut lines = non_empty_lines(stderr);
    let error = lines
        .iter()
        .position(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .map(|at| lines.remove(at));
    let others = MAX_LINES - usize::from(error.is_some());
    lines.drain(..lines.len().saturating_sub(others));
    error.into_iter().chain(lines).collect()
}

/// The non-empty lines of `stream`, trimmed.
fn non_empty_lines(stream: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stream)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {

    use super::*;

    /// A repository in a temp directory, with an identity and one commit, and
    /// a bare repository beside it as its `origin`. No global or system
    /// config is read.
    pub(super) fn repo_with_origin() -> (tempfile::TempDir, Git) {
        let temp = tempfile::TempDir::new().unwrap();
        let isolated = |dir: &Path, args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        isolated(temp.path(), &["init", "-q", "--bare", "origin.git"]);
        isolated(temp.path(), &["init", "-q", "-b", "main", "work"]);
        let work = temp.path().join("work");
        for (key, value) in [
            ("user.name", "Test Runner"),
            ("user.email", "runner@example.com"),
            ("remote.origin.url", "../origin.git"),
        ] {
            isolated(&work, &["config", key, value]);
        }
        isolated(&work, &["commit", "-q", "--allow-empty", "-m", "Initial"]);
        (temp, Git::new(work))
    }

    #[test]
    fn a_failure_includes_a_hooks_output_and_gits_own_error() {
        let (_temp, git) = repo_with_origin();
        let hook = git.dir().join(".git/hooks/pre-push");
        crate::test_support::write_executable(&hook, "#!/bin/sh\necho 'hook says no'\nexit 1\n");

        let error = git
            .run(&["push", "origin", "main"])
            .unwrap_err()
            .to_string();

        assert!(error.contains("hook says no"), "{error}");
        assert!(error.contains("failed to push some refs"), "{error}");
    }

    #[test]
    fn a_failure_starts_with_gits_own_error_line() {
        let (_temp, git) = repo_with_origin();
        let alias = "alias.fetch-main=!printf '%s\\n' \
                     'From https://github.com/acme/widgets' \
                     ' * branch            main       -> FETCH_HEAD' \
                     '   93d8147..8cf48f0  main       -> origin/main' \
                     'error: some local refs could not be updated' \
                     'error: try again' >&2; \
                     echo fetched; exit 1";

        let error = git
            .run(&["-c", alias, "fetch-main"])
            .unwrap_err()
            .to_string();

        let lines: Vec<&str> = error.lines().collect();
        assert!(
            lines[0].ends_with(" fetch-main failed: error: some local refs could not be updated"),
            "{error}"
        );
        assert_eq!(
            lines[1..],
            [
                "From https://github.com/acme/widgets",
                "* branch            main       -> FETCH_HEAD",
                "93d8147..8cf48f0  main       -> origin/main",
                "error: try again",
                "fetched",
            ]
        );
    }

    #[test]
    fn a_failure_includes_stdout() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&["-c", "alias.fail=!echo $((6 * 7)); exit 1", "fail"])
            .unwrap_err()
            .to_string();

        assert!(error.ends_with(": 42"), "{error}");
    }

    #[test]
    fn a_failure_keeps_only_the_last_lines_of_a_stream() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&["-c", "alias.fail=!seq 1 100; exit 1", "fail"])
            .unwrap_err()
            .to_string();

        assert!(error.contains("\n100"), "{error}");
        assert!(!error.contains("\n50\n"), "{error}");
    }

    #[test]
    fn gits_own_error_line_is_kept_however_many_lines_follow_it() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&[
                "-c",
                "alias.fail=!{ echo 'fatal: no'; seq 1 100; } >&2; exit 1",
                "fail",
            ])
            .unwrap_err()
            .to_string();

        let lines: Vec<&str> = error.lines().collect();
        assert!(lines[0].ends_with(" fail failed: fatal: no"), "{error}");
        assert_eq!(
            lines[1..],
            ["92", "93", "94", "95", "96", "97", "98", "99", "100"]
        );
    }

    #[test]
    fn a_lock_another_git_holds_is_waited_for() {
        let commands: [(&str, &[&str]); 2] = [
            (".git/config.lock", &["config", "thirdshift.test", "yes"]),
            (
                ".git/refs/heads/other.lock",
                &["update-ref", "refs/heads/other", "HEAD"],
            ),
        ];
        for (lock, args) in commands {
            let (_temp, git) = repo_with_origin();
            let lock = git.dir().join(lock);
            std::fs::write(&lock, "").unwrap();
            let released = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                std::fs::remove_file(lock).unwrap();
            });

            let result = git.run(args);

            released.join().unwrap();
            result.unwrap();
        }
    }

    #[test]
    fn a_ref_another_git_moved_meanwhile_is_tried_again() {
        // The race as older git words it, then as git 2.52 does.
        for error in [
            "error: cannot lock ref 'refs/remotes/origin/main': is at 1 but expected 2",
            "error: fetching ref refs/remotes/origin/main failed: incorrect old value provided",
        ] {
            let (temp, git) = repo_with_origin();
            let moved = temp.path().join("moved");
            let alias = format!(
                "alias.race=!test -f {0} && exit 0; touch {0}; echo \"{error}\" >&2; exit 1",
                moved.display()
            );

            git.run(&["-c", &alias, "race"]).unwrap();
        }
    }

    #[test]
    fn a_fetch_still_reports_a_missing_required_ref() {
        let (_temp, git) = repo_with_origin();
        git.push("main").unwrap();

        let error = git.fetch(&["absent"]).unwrap_err().to_string();

        assert!(error.contains("couldn't find remote ref absent"), "{error}");
    }

    #[test]
    fn a_fetch_still_reports_a_transport_failure() {
        let (_temp, git) = repo_with_origin();
        git.run(&["config", "remote.origin.url", "../absent.git"])
            .unwrap();

        let error = git.fetch(&["main"]).unwrap_err().to_string();

        assert!(
            error.contains("does not appear to be a git repository"),
            "{error}"
        );
    }

    #[test]
    fn a_fetch_still_reports_a_missing_required_object() {
        let (temp, git) = repo_with_origin();
        let initial = git.run(&["rev-parse", "HEAD"]).unwrap();
        git.run(&["commit", "-q", "--allow-empty", "-m", "Required commit"])
            .unwrap();
        let required = git.run(&["rev-parse", "HEAD"]).unwrap();
        git.push("main").unwrap();
        git.run(&["reset", "--hard", &initial]).unwrap();
        for objects in [
            git.dir().join(".git/objects"),
            temp.path().join("origin.git/objects"),
        ] {
            std::fs::remove_file(objects.join(&required[..2]).join(&required[2..])).unwrap();
        }

        let error = git.fetch(&["main"]).unwrap_err().to_string();

        assert!(
            error.contains("git fetch") && error.contains("failed"),
            "{error}"
        );
        assert!(error.contains(&required), "{error}");
    }

    #[test]
    fn a_failure_with_nothing_on_either_stream_says_only_that_git_failed() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&["-c", "alias.fail=!exit 1", "fail"])
            .unwrap_err()
            .to_string();

        assert_eq!(error, "git -c alias.fail=!exit 1 fail failed");
    }
}
