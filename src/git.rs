//! Running `git` in a directory.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// `git` bound to a working directory. Output is captured, never passed
/// through, so stdout stays reserved for the PR URL.
pub struct Git {
    dir: PathBuf,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Git { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run `git <args>` and return its trimmed stdout. If it exits non-zero,
    /// fail with git's own `error:` or `fatal:` line, if it wrote one, then
    /// the last lines of its stderr and of its stdout, where a hook's
    /// explanation can end up. While it fails on a lock file another git
    /// holds, as when a Spec run's Tickets fetch or create worktrees from
    /// one Launch directory at once, it is run again, for up to
    /// [`LOCK_WAIT`].
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let deadline = Instant::now() + LOCK_WAIT;
        let mut output = self.output(args)?;
        while !output.status.success() && held_lock(&output.stderr) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            output = self.output(args)?;
        }
        if !output.status.success() {
            let tail = [cause_first(&output.stderr), last_lines(&output.stdout)].concat();
            let mut message = format!("git {} failed", args.join(" "));
            if !tail.is_empty() {
                message = format!("{message}: {}", tail.join("\n"));
            }
            bail!(message);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// The repository's common git directory, the one its worktrees share.
    pub fn common_dir(&self) -> Result<PathBuf> {
        Ok(self.dir.join(self.run(&["rev-parse", "--git-common-dir"])?))
    }

    /// Wait for, then hold until the file returned is dropped, a lock on the
    /// file `name` in the repository's common git directory, which every Run
    /// from one Launch directory shares.
    pub fn lock(&self, name: &str) -> Result<File> {
        let path = self.common_dir()?.join(name);
        let file = File::create(&path).with_context(|| format!("can't open {}", path.display()))?;
        file.lock()
            .with_context(|| format!("can't lock {}", path.display()))?;
        Ok(file)
    }

    /// Whether origin has the branch `branch`, as origin itself answers.
    pub fn on_origin(&self, branch: &str) -> Result<bool> {
        let reference = format!("refs/heads/{branch}");
        let found = self.run(&["ls-remote", "--heads", "origin", &reference])?;
        Ok(!found.is_empty())
    }

    /// Run `git <args>` and report whether it exited zero, for commands whose
    /// exit status is the answer.
    pub fn succeeds(&self, args: &[&str]) -> Result<bool> {
        Ok(self.output(args)?.status.success())
    }

    fn output(&self, args: &[&str]) -> Result<Output> {
        Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .with_context(|| format!("could not run git {}", args.join(" ")))
    }
}

/// How long [`Git::run`] keeps trying a command that fails on a lock file.
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
    let mut lines = lines(stream);
    lines.drain(..lines.len().saturating_sub(MAX_LINES));
    lines
}

/// The lines of a failed git's `stderr` to report, `MAX_LINES` at most: the
/// first line starting `error:` or `fatal:`, which says what went wrong,
/// then the last of the others. With no such line, its last lines. So a
/// reader of the first line alone, as of a child Run's cause, reads git's
/// error and not a banner such as a fetch's `From <url>`.
fn cause_first(stderr: &[u8]) -> Vec<String> {
    let mut lines = lines(stderr);
    let cause = lines
        .iter()
        .position(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .map(|at| lines.remove(at));
    let others = MAX_LINES - cause.iter().count();
    lines.drain(..lines.len().saturating_sub(others));
    cause.into_iter().chain(lines).collect()
}

/// The non-empty lines of `stream`, trimmed.
fn lines(stream: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stream)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A repository in a temp directory, with an identity and one commit, and
    /// a bare repository beside it as its `origin`. No global or system
    /// config is read.
    fn repo_with_origin() -> (tempfile::TempDir, Git) {
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
        std::fs::write(&hook, "#!/bin/sh\necho 'hook says no'\nexit 1\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

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
    fn a_failure_with_nothing_on_either_stream_says_only_that_git_failed() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&["-c", "alias.fail=!exit 1", "fail"])
            .unwrap_err()
            .to_string();

        assert_eq!(error, "git -c alias.fail=!exit 1 fail failed");
    }
}
