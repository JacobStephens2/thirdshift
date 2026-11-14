//! Version-1 Custom runner requests and per-Trial result classification.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Output as ProcessOutput;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;

use super::super::{Task, arms, store};
use crate::process::{self, Control, Interruption};
use crate::trial::{Outcome, Output, VoidCause};

pub(super) fn prepare(
    runner: &arms::Runner,
    task: &Task,
    repository: &Path,
    attempt: &Path,
) -> Result<()> {
    let runner_dir = runner.install(attempt)?;
    let inputs = task
        .manifest
        .trial
        .as_ref()
        .context("missing Trial inputs")?;
    fs::create_dir_all(attempt.join("session"))?;
    let git = crate::git::Git::new(repository);
    crate::skills::exclude(&git, &format!("/{}", crate::prompt::ISSUE_FILE))?;
    let exclude = git.run(&["rev-parse", "--git-path", "info/exclude"])?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(repository.join(exclude))?;
    writeln!(file)?;
    for pattern in &runner.manifest.git_exclude {
        writeln!(file, "{pattern}")?;
    }
    store::write(
        &attempt.join("request.json"),
        &json!({
            "version": 1, "repository": repository, "issue_file": repository.join(crate::prompt::ISSUE_FILE),
            "issue_url": inputs.issue_url, "base": inputs.base, "branch": inputs.branch,
            "runner_dir": runner_dir, "output_dir": attempt.join("session"),
            // #2191 supplies the live recorder; retaining the field makes the
            // version-1 request usable by runners that do not record CLI sessions.
            "record_command": null
        }),
    )
}

pub(super) fn command(runner: &arms::Runner, attempt: &Path) -> Result<Vec<String>> {
    let directory = attempt.join("runner");
    let directory = directory
        .to_str()
        .context("runner directory must be UTF-8")?;
    let mut command: Vec<_> = runner
        .manifest
        .command
        .iter()
        .map(|arg| arg.replace("{runner_dir}", directory))
        .collect();
    command.push(
        attempt
            .join("request.json")
            .to_str()
            .context("runner request must be UTF-8")?
            .into(),
    );
    Ok(command)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDocument {
    version: u64,
    outcome: Outcome,
    #[serde(default)]
    void_cause: Option<VoidCause>,
    #[serde(default)]
    cause: Option<String>,
}

/// Runner mistakes fail one Trial. Outer supervision classifies its own
/// evidence before this function, and may override it after collection.
pub(super) fn result(process: &ProcessOutput, attempt: &Path, elapsed: u128) -> Output {
    let result = (|| -> Result<Output> {
        let document: ResultDocument = serde_json::from_slice(
            &fs::read(attempt.join("session/result.json"))
                .context("missing or unreadable Custom runner result.json")?,
        )
        .context("invalid Custom runner result.json")?;
        ensure!(
            document.version == 1,
            "unsupported Custom runner result version"
        );
        ensure!(
            document.outcome != Outcome::Succeeded,
            "Custom runner outcome must be delivered, stopped_to_ask, failed or void"
        );
        if let Some(cause) = document.void_cause {
            ensure!(
                matches!(
                    cause,
                    VoidCause::SessionUnavailable | VoidCause::UsageLimit | VoidCause::RateLimit
                ),
                "Custom runner claimed outer void_cause {cause}"
            );
            ensure!(
                document.outcome == Outcome::Void,
                "void_cause requires a void outcome"
            );
        }
        ensure!(
            document.outcome != Outcome::Void || document.void_cause.is_some(),
            "Custom runner void requires void_cause"
        );
        ensure!(
            process.status.success(),
            "Custom runner exited nonzero without outer Void evidence"
        );
        let mut output = Output::failed(elapsed, "Custom runner failed");
        output.outcome = document.outcome;
        output.void_cause = document.void_cause;
        output.cause = document.cause;
        if output.outcome == Outcome::Failed {
            output.cause = Some(diagnostic(
                process,
                output.cause.as_deref().unwrap_or("Custom runner failed"),
            ));
        }
        Ok(output)
    })();
    result.unwrap_or_else(|error| {
        Output::failed(elapsed, &diagnostic(process, &format!("{error:#}")))
    })
}

fn diagnostic(process: &ProcessOutput, cause: &str) -> String {
    format!(
        "{cause}; exit status: {}; stderr: {}",
        process.status,
        String::from_utf8_lossy(&process.stderr)
    )
}

pub(super) fn inspect(
    runner: &arms::Runner,
    repository: &Path,
    attempt: &Path,
    deadline: Instant,
) -> Result<Option<String>> {
    let paths = crate::evals::repository::bytes_until(
        repository,
        &["ls-tree", "-rz", "--name-only", "HEAD"],
        deadline,
    )?;
    // Evaluate only the frozen manifest's patterns, independent of the
    // runner's edits to .gitignore, its index or .git/info/exclude.
    let scratch = tempfile::tempdir()?;
    crate::git::Git::new(scratch.path()).run(&["init", "-q", "--template="])?;
    fs::create_dir_all(scratch.path().join(".git/info"))?;
    fs::write(
        scratch.path().join(".git/info/exclude"),
        runner.manifest.git_exclude.join("\n") + "\n",
    )?;
    let matches = process::output_until(
        std::process::Command::new("git")
            .current_dir(scratch.path())
            .args([
                "-c",
                "core.excludesFile=/dev/null",
                "check-ignore",
                "--no-index",
                "-z",
                "--stdin",
            ]),
        Some(&paths),
        Control {
            name: "Custom runner HEAD check",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        deadline,
        &|| Ok(()),
    )?;
    ensure!(
        matches.status.code() == Some(1) || matches.status.success(),
        "could not check Custom runner git_exclude: {}",
        String::from_utf8_lossy(&matches.stderr)
    );
    ensure!(
        matches.stdout.is_empty(),
        "Custom runner committed a git_exclude path on HEAD: {}",
        String::from_utf8_lossy(&matches.stdout).replace('\0', ", ")
    );
    let status = crate::evals::repository::bytes_until(
        repository,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=traditional",
        ],
        deadline,
    )?;
    if status.is_empty() {
        return Ok(None);
    }
    fs::write(attempt.join("uncommitted.txt"), &status)?;
    Ok(Some(format!(
        "uncommitted working-tree files ignored by grading: {}",
        String::from_utf8_lossy(&status).trim()
    )))
}
