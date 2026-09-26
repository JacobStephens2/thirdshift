//! Running `git` in a directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
    /// fail with the last lines of its stderr and then of its stdout, where a
    /// hook's explanation can end up.
    pub fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.output(args)?;
        if !output.status.success() {
            let tail: Vec<String> = [&output.stderr, &output.stdout]
                .into_iter()
                .flat_map(|stream| last_lines(stream))
                .collect();
            let mut message = format!("git {} failed", args.join(" "));
            if !tail.is_empty() {
                message = format!("{message}: {}", tail.join("\n"));
            }
            bail!(message);
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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

/// The most lines of each stream a failure reports.
const MAX_LINES: usize = 10;

/// The last `MAX_LINES` non-empty lines of `stream`, trimmed.
fn last_lines(stream: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(stream);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines[lines.len().saturating_sub(MAX_LINES)..]
        .iter()
        .map(|line| line.to_string())
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
    fn a_failure_with_nothing_on_either_stream_says_only_that_git_failed() {
        let (_temp, git) = repo_with_origin();

        let error = git
            .run(&["-c", "alias.fail=!exit 1", "fail"])
            .unwrap_err()
            .to_string();

        assert_eq!(error, "git -c alias.fail=!exit 1 fail failed");
    }
}
