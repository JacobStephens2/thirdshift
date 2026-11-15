//! Select green, unreleased content without consulting the Launch tree.
use std::time::Instant;

use anyhow::{Result, ensure};

use super::{Version, fetch, git_run};
use crate::{git::Git, github::GitHub, poll, progress};

pub(super) enum Candidate {
    Ready { base: String },
    Skipped(String),
}

pub(super) fn candidate(
    git: &Git,
    github: &GitHub,
    wanted: Option<&Version>,
    previous: Option<&str>,
) -> Result<Candidate> {
    let mut line = poll::WaitingLine::new();
    let started = Instant::now();
    loop {
        fetch(git)?;
        let boundary = previous.map(|tag| format!("^{tag}"));
        let mut args = vec!["rev-list", "--first-parent", "origin/main"];
        if let Some(boundary) = &boundary {
            args.push(boundary);
        }
        let commits = git_run(git, &args)?;
        let mut eligible = false;
        let mut pending = false;
        let mut failure = String::new();
        for sha in commits.lines() {
            if let Some(previous) = previous {
                // Release tags may sit on a merged side branch of main.
                if git_run(git, &["merge-base", previous, sha])?
                    != git_run(git, &["rev-parse", &format!("{previous}^{{commit}}")])?
                {
                    continue;
                }
            }
            if wanted.is_none() && !shipped_changes(git, previous, sha)? {
                continue;
            }
            eligible = true;
            match github.release_push_run("ci.yml", "main", sha)? {
                Some(run)
                    if run.status == "completed"
                        && run.conclusion.as_deref() == Some("success") =>
                {
                    return Ok(Candidate::Ready { base: sha.into() });
                }
                Some(run) if run.status != "completed" => pending = true,
                None if started.elapsed() < poll::grace_period() => pending = true,
                run => {
                    if failure.is_empty() {
                        failure = format!(
                            "main push CI (ci.yml) at {sha}: {}",
                            run.and_then(|run| run.conclusion)
                                .unwrap_or_else(|| "no CI run".into())
                        );
                    }
                }
            }
        }
        if wanted.is_none() && !eligible {
            return Ok(Candidate::Skipped(if commits.is_empty() {
                "nothing changed since the last release tag".into()
            } else {
                "no shipped changes since the last release tag".into()
            }));
        }
        if !pending && wanted.is_none() {
            return Ok(Candidate::Skipped(format!(
                "no candidate main commit has successful push CI; {failure}"
            )));
        }
        ensure!(
            pending,
            "no candidate main commit has successful push CI; all candidates completed non-green or have no CI"
        );
        if line.should_report(()) {
            // Admission skips stay silent; once CI is pending, the wait must
            // be visible before a candidate turns green, in Working and to Stop.
            crate::logs::waiting();
            crate::logs::show_held();
            progress::step("waiting for main push CI before releasing");
        }
        poll::pause()?;
    }
}

fn shipped_changes(git: &Git, previous: Option<&str>, sha: &str) -> Result<bool> {
    let paths = if let Some(previous) = previous {
        git_run(
            git,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--name-only",
                "--no-renames",
                "-z",
                previous,
                sha,
                "--",
            ],
        )?
    } else {
        git_run(git, &["ls-tree", "-r", "--name-only", "-z", sha])?
    };
    Ok(paths.split('\0').any(|path| {
        // Console templates and assets under src/ are compiled into the binary too.
        path.starts_with("src/")
            || path.starts_with("skills/")
            || path.starts_with("processes/")
            || path.starts_with(".cargo/")
            || matches!(
                path,
                "Cargo.toml"
                    | "Cargo.lock"
                    | "dist-workspace.toml"
                    | "rust-toolchain.toml"
                    | "rust-toolchain"
                    | "README.md"
                    | "LICENSE"
                    | "askama.toml"
            )
            || path.starts_with(".github/workflows/release")
            || path.starts_with(".github/workflows/publish")
            || path.starts_with("scripts/release")
    }))
}
