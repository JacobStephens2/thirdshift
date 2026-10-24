//! Proposed Bug origins retain evidence; confirmation and metrics belong to #1029.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::UserConfig;
use crate::git::Git;
use crate::github::{GitHub, scoring::origins::ClosedBug};
use crate::harness::built_with;
use crate::session::recording::{Input, Recording};
use crate::{interrupt, logs, prompt};

#[derive(Serialize, Deserialize)]
pub(super) struct Proposal {
    issue: String,
    closed_at: DateTime<Utc>,
    state: String,
    commit: Option<String>,
    reason: String,
    pull_requests: Vec<PullRequest>,
}

#[derive(Serialize, Deserialize)]
struct PullRequest {
    url: String,
    run: Option<Run>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum Run {
    Local {
        command_record: PathBuf,
        name: String,
    },
    BuiltWith {
        harness: String,
        version: Option<String>,
        model: String,
        effort: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    commit: Option<String>,
    reason: String,
}

pub(super) fn propose(
    config: &UserConfig,
    start: Option<DateTime<Utc>>,
    end: DateTime<Utc>,
    directory: &Path,
) -> Result<Vec<Proposal>> {
    let github = GitHub::new();
    let bugs = github.closed_bugs(&config.scoring.owners, start, end)?;
    let mut proposals = Vec::new();
    for (index, bug) in bugs.into_iter().enumerate() {
        interrupt::check()?;
        let repository = bug.repository()?;
        let response = match bug.fix() {
            Some(fix) => analyze(config, &bug, fix, &directory.join(index.to_string()))?,
            None => Response { commit: None, reason: "GitHub records no fix commit for this closure; history cannot establish an introducing commit.".into() },
        };
        let mut pull_requests = Vec::new();
        if let Some(commit) = &response.commit {
            for pr in github.origin_pull_requests(&repository, commit)? {
                // A later PR carrying the same commit cannot have introduced
                // a bug already closed. Keep all historical associations,
                // rather than guessing between several merged PRs.
                if !pr.merged_at.is_some_and(|merged| merged <= bug.closed_at) {
                    continue;
                }
                let run = if let Some((command_record, name)) =
                    logs::run_for_pull_request(&config.logs_dir, &pr.html_url)?
                {
                    Some(Run::Local {
                        command_record,
                        name,
                    })
                } else {
                    built_with::read(pr.body.as_deref().unwrap_or_default()).map(|built| {
                        Run::BuiltWith {
                            harness: built.harness.name().into(),
                            version: built.version,
                            model: built.model,
                            effort: built.effort,
                        }
                    })
                };
                pull_requests.push(PullRequest {
                    url: pr.html_url,
                    run,
                });
            }
        }
        proposals.push(Proposal {
            issue: bug.url,
            closed_at: bug.closed_at,
            state: "proposed".into(),
            commit: response.commit,
            reason: response.reason,
            pull_requests,
        });
    }
    Ok(proposals)
}

fn analyze(config: &UserConfig, bug: &ClosedBug, fix: &str, directory: &Path) -> Result<Response> {
    ensure!(is_commit(fix), "Bug-origin fix is not a full commit ID");
    let workspace = directory.join("workspace");
    let repository = workspace.join("repository");
    fs::create_dir_all(&repository)?;
    let git = Git::new(&repository);
    git.run(&["init", "--quiet", "--template="])?;
    git.run(&[
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        "--no-recurse-submodules",
        &format!("https://github.com/{}.git", bug.repository()?),
        fix,
    ])?;
    git.run(&["checkout", "--detach", fix])?;
    // Capture allowed identities before the writable session starts. No
    // invented object or ref the agent creates can become historical evidence.
    let history: BTreeSet<_> = git
        .run(&["rev-list", fix])?
        .lines()
        .map(str::to_owned)
        .collect();
    fs::write(
        workspace.join("input.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "issue": bug.url, "title": bug.title, "body": bug.body, "fix": fix,
        }))?,
    )?;
    let prompt = prompt::bug_origin();
    fs::write(workspace.join("prompt.md"), &prompt)?;
    crate::resources::wait()?;
    let choice = config.scoring.grader();
    let adapter = choice.harness.adapter();
    let binary = crate::sandbox::resolve_binary(choice.harness.name())?;
    let invocation = adapter.trial_session(&choice, None, &prompt);
    let sandbox = crate::sandbox::for_session(&choice, &binary, &workspace, directory)?;
    let caps = config.trial_caps.with_env()?;
    let mut launched = match &sandbox {
        Some(prepared) => prepared
            .launch()
            .scoped_command(&invocation.args, &workspace, &caps)?,
        None => crate::scope::grader_command(&binary, &invocation.args, &workspace, &caps)?,
    };
    launched
        .command
        .stderr(fs::File::create(directory.join("stderr"))?);
    let ended = Recording::for_trial().run(
        "Bug-origin proposal",
        &choice,
        &mut launched.command,
        Input {
            prompt: &prompt,
            stdin: invocation.stdin.as_deref(),
            deadline: Some(Instant::now() + Duration::from_secs(300)),
            private: true,
        },
        &directory.join("session.jsonl"),
        adapter.interpretation(&workspace, &prompt),
    );
    interrupt::check()?;
    let ended = ended.map_err(|error| {
        if error.is::<crate::session::limits::Reached>() {
            anyhow::anyhow!("Scoring stopped at a plan-limit warning; window was not advanced")
        } else {
            error.context("Bug-origin session failed")
        }
    })?;
    ensure!(
        ended.killed.is_empty(),
        "Bug-origin session left unfinished background work"
    );
    let response: Response = serde_json::from_slice(
        &fs::read(workspace.join("proposal.json"))
            .context("Bug-origin session left no proposal")?,
    )?;
    ensure!(
        !response.reason.trim().is_empty(),
        "Bug-origin proposal has no evidence"
    );
    if let Some(commit) = &response.commit {
        ensure!(
            is_commit(commit) && commit != fix && history.contains(commit),
            "Bug-origin commit is not in pre-fix history"
        );
    }
    Ok(response)
}

fn is_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}
