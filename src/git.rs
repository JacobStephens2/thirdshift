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
mod transport;

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
    /// it is run again, for up to [`LOCK_WAIT`]. An `ls-remote` or `fetch`
    /// that fails to reach origin in transport is run again, up to
    /// [`TRANSPORT_ATTEMPTS`] times in all, after a progress line each time.
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
        command_result(args, &output)
    }

    /// A destructive transition has different authority before and after
    /// success. Failed attempts (including execution errors) still need an
    /// outcome check before retrying, with the original failure preserved.
    pub(crate) fn run_transition(
        &self,
        args: &[&str],
        before: impl Fn() -> Result<()>,
        after: impl Fn(&Result<Output>) -> Result<()>,
    ) -> Result<String> {
        let output = Self::retry_output(|| {
            before()?;
            let result = self.output(args);
            if let Err(authority) = after(&result) {
                let failure = match &result {
                    Ok(output) => command_result(args, output).err(),
                    Err(error) => Some(anyhow::anyhow!("{error:#}")),
                };
                return match failure {
                    Some(failure) => Err(authority).context(format!("{failure:#}")),
                    None => Err(authority),
                };
            }
            result
        })?;
        command_result(args, &output)
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
            Self::retry_output(|| {
                check()?;
                let result = self.output(args);
                check()?;
                result
            })
        };
        if !reads_origin(args) {
            return attempt();
        }
        // A read of origin that a dropped connection failed is tried again,
        // after one then two seconds. Pushes and other mutations never are.
        for tried in 1..=TRANSPORT_ATTEMPTS {
            let output = attempt()?;
            if output.status.success()
                || tried == TRANSPORT_ATTEMPTS
                || !transport::is_transient(&output.stderr)
            {
                return Ok(output);
            }
            let cause = command_result(args, &output).unwrap_err();
            let cause = format!("{cause:#}");
            let cause = cause.lines().next().unwrap_or_default();
            progress::step(format_args!(
                "{cause} (attempt {tried}/{TRANSPORT_ATTEMPTS}); retrying in {tried}s"
            ));
            let delay = Duration::from_secs(u64::from(tried));
            match self.interruption {
                Interruption::Ordinary => crate::poll::sleep(delay)?,
                Interruption::Completion => std::thread::sleep(delay),
            }
        }
        unreachable!("the last attempt returns its output")
    }

    fn retry_output(attempt: impl Fn() -> Result<Output>) -> Result<Output> {
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

    /// The checkout's top level, administrative and common directories and
    /// verified HEAD, read by one process because spawning git is the cost
    /// of observing them. Each is trimmed as [`Git::run`] trims. Output that
    /// is not four lines, as from a path with a newline in it, is observed
    /// again one value per process, so no path is guessed at.
    pub fn checkout_location(&self) -> Result<CheckoutLocation> {
        let observed = self.run(&[
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
            "--verify",
            "HEAD",
        ])?;
        let [root, admin, common, head] = match observed.lines().collect::<Vec<_>>()[..] {
            [root, admin, common, head] => {
                [root, admin, common, head].map(|line| line.trim().to_string())
            }
            _ => [
                self.run(&["rev-parse", "--show-toplevel"])?,
                self.run(&["rev-parse", "--absolute-git-dir"])?,
                self.run(&["rev-parse", "--git-common-dir"])?,
                self.run(&["rev-parse", "--verify", "HEAD"])?,
            ],
        };
        Ok(CheckoutLocation {
            root: PathBuf::from(root),
            admin: PathBuf::from(admin),
            common: self.dir.join(common),
            head,
        })
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
    /// The caller proves `expected_head` is an ancestor of the pinned `head`.
    /// The lease guards deletion/replacement; it never authorizes discarding history.
    pub(crate) fn push_checked(
        &self,
        branch: &str,
        head: &str,
        expected_head: Option<&str>,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let lease = format!(
            "--force-with-lease=refs/heads/{branch}:{}",
            expected_head.unwrap_or("")
        );
        let refspec = format!("{head}:refs/heads/{branch}");
        progress::step(format_args!("pushing {branch}"));
        self.run_checked(&["push", "--no-verify", &lease, "origin", &refspec], check)?;
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

fn command_result(args: &[&str], output: &Output) -> Result<String> {
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

/// Where a checkout is, as [`Git::checkout_location`] reads it.
pub struct CheckoutLocation {
    pub root: PathBuf,
    pub admin: PathBuf,
    pub common: PathBuf,
    pub head: String,
}

/// How many times a read of origin is run while it fails in transport.
const TRANSPORT_ATTEMPTS: u32 = 3;

/// Whether `args` is a git command that only reads origin over the network,
/// `ls-remote` or `fetch`, after any of git's own options such as `-c`.
fn reads_origin(args: &[&str]) -> bool {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-c" | "-C" => {
                args.next();
            }
            option if option.starts_with('-') => {}
            command => return matches!(command, "ls-remote" | "fetch"),
        }
    }
    false
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
        git.run(&["push", "--no-verify", "origin", "main"]).unwrap();

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
        git.run(&["push", "--no-verify", "origin", "main"]).unwrap();
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

    /// Make origin's `pack` side (`uploadpack`, which serves `ls-remote` and
    /// `fetch`, or `receivepack`, which serves `push`) write `stderr` and
    /// fail its first `fails` connections. The file returned counts them.
    pub(super) fn origin_fails(
        temp: &tempfile::TempDir,
        git: &Git,
        pack: &str,
        fails: u32,
        stderr: &str,
    ) -> PathBuf {
        let count = temp.path().join(format!("{pack}-connections"));
        let script = temp.path().join(pack);
        let serve = if pack == "uploadpack" {
            "upload-pack"
        } else {
            "receive-pack"
        };
        crate::test_support::write_executable(
            &script,
            &format!(
                "#!/bin/sh\n\
                 n=$(($(cat '{count}' 2>/dev/null || echo 0) + 1))\n\
                 echo $n > '{count}'\n\
                 if [ $n -le {fails} ]; then printf '%s\\n' '{stderr}' >&2; exit 128; fi\n\
                 exec git {serve} \"$@\"\n",
                count = count.display(),
            ),
        );
        git.run(&[
            "config",
            &format!("remote.origin.{pack}"),
            &script.display().to_string(),
        ])
        .unwrap();
        count
    }

    fn connections(count: &Path) -> u32 {
        std::fs::read_to_string(count)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    const DROPPED_STREAM: &str = "error: RPC failed; curl 16 Error in the HTTP2 framing layer";

    #[test]
    fn an_ls_remote_a_dropped_stream_failed_is_run_again() {
        let (temp, git) = repo_with_origin();
        git.run(&["push", "--no-verify", "origin", "main"]).unwrap();
        let count = origin_fails(&temp, &git, "uploadpack", 1, DROPPED_STREAM);

        assert!(git.on_origin("main").unwrap());
        assert_eq!(connections(&count), 2);
    }

    #[test]
    fn a_fetch_a_dropped_stream_failed_is_run_again() {
        let (temp, git) = repo_with_origin();
        git.run(&["push", "--no-verify", "origin", "main"]).unwrap();
        let count = origin_fails(&temp, &git, "uploadpack", 1, DROPPED_STREAM);

        git.fetch(&["main"]).unwrap();

        assert_eq!(connections(&count), 2);
        git.run(&["rev-parse", "--verify", "FETCH_HEAD"]).unwrap();
    }

    #[test]
    fn a_read_of_origin_failing_in_transport_throughout_fails_with_the_last_cause() {
        let (temp, git) = repo_with_origin();
        let count = origin_fails(&temp, &git, "uploadpack", 3, DROPPED_STREAM);
        let started = Instant::now();

        let error = git.on_origin("main").unwrap_err().to_string();

        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(connections(&count), 3);
        assert!(
            error.starts_with(&format!(
                "git ls-remote --heads origin refs/heads/main failed: {DROPPED_STREAM}"
            )),
            "{error}"
        );
    }

    #[test]
    fn a_read_of_origin_failing_permanently_is_run_once() {
        for stderr in [
            "fatal: Authentication failed for https://github.com/acme/widgets/",
            "remote: Repository not found.",
            "fatal: something nobody has classified",
        ] {
            let (temp, git) = repo_with_origin();
            let count = origin_fails(&temp, &git, "uploadpack", 3, stderr);

            let error = git.fetch(&["main"]).unwrap_err().to_string();

            assert_eq!(connections(&count), 1, "{stderr}");
            assert!(error.contains(stderr), "{error}");
        }
    }

    #[test]
    fn a_push_failing_in_transport_is_attempted_once() {
        let (temp, git) = repo_with_origin();
        let count = origin_fails(&temp, &git, "receivepack", 1, DROPPED_STREAM);
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();

        let error = git
            .push_checked("main", &head, None, || Ok(()))
            .unwrap_err()
            .to_string();

        assert_eq!(connections(&count), 1);
        assert!(error.contains(DROPPED_STREAM), "{error}");
    }

    #[test]
    fn only_ls_remote_and_fetch_read_origin() {
        assert!(reads_origin(&["ls-remote", "--heads", "origin"]));
        assert!(reads_origin(&[
            "-c",
            "http.version=HTTP/2",
            "fetch",
            "origin"
        ]));
        assert!(!reads_origin(&["push", "origin", "main"]));
        assert!(!reads_origin(&["-c", "alias.fetch=x", "status"]));
        assert!(!reads_origin(&["rev-parse", "--verify", "fetch"]));
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
