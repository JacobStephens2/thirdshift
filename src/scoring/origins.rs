//! Bug origins retain the proposed evidence beside the Day shift's settlement.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::UserConfig;
use crate::git::Git;
use crate::github::{GitHub, scoring::origins::ClosedBug};
use crate::harness::built_with;
use crate::session::private;
use crate::{interrupt, logs, prompt};

mod settlement;
pub(crate) use settlement::{Options, apply, execute, parse, view};

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Proposed,
    Confirmed,
    Rejected,
}

impl State {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Proposal {
    pub issue: String,
    pub closed_at: DateTime<Utc>,
    pub state: State,
    pub commit: Option<String>,
    pub reason: String,
    pub pull_requests: Vec<PullRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<Settlement>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Settlement {
    pub pull_request: PullRequest,
    pub corrected: bool,
    pub eval_task: crate::evals::bug_origin::Reference,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct PullRequest {
    pub url: String,
    run: Option<Run>,
}

impl PullRequest {
    pub fn attribution(&self) -> String {
        match &self.run {
            Some(Run::Local { name, .. }) => format!("Ticket run: {name}"),
            Some(Run::BuiltWith {
                harness,
                version,
                model,
                effort,
            }) => format!(
                "{} {} · {} · {}",
                harness,
                version.as_deref().unwrap_or("version unknown"),
                model,
                effort
            ),
            None => "No retained Ticket run".into(),
        }
    }

    fn attributed(logs_dir: &Path, url: String, body: &str) -> Result<Self> {
        let run = if let Some((command_record, name)) = logs::run_for_pull_request(logs_dir, &url)?
        {
            Some(Run::Local {
                command_record,
                name,
            })
        } else {
            built_with::read(body).map(|built| Run::BuiltWith {
                harness: built.harness.name().into(),
                version: built.version,
                model: built.model,
                effort: built.effort,
            })
        };
        Ok(Self { url, run })
    }
}

#[derive(Clone, Serialize, Deserialize)]
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
                pull_requests.push(PullRequest::attributed(
                    &config.logs_dir,
                    pr.html_url,
                    pr.body.as_deref().unwrap_or_default(),
                )?);
            }
        }
        proposals.push(Proposal {
            issue: bug.url,
            closed_at: bug.closed_at,
            state: State::Proposed,
            commit: response.commit,
            reason: response.reason,
            pull_requests,
            settlement: None,
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
    let choice = config.scoring.grader();
    let binary = crate::sandbox::resolve_binary(choice.harness.name())?;
    let caps = config.trial_caps.with_env()?;
    let ran = private::run(private::Request {
        kind: "Bug-origin proposal",
        choice: &choice,
        binary: &binary,
        workspace: &workspace,
        artifacts: directory,
        prompt: &prompt,
        timeout: Duration::from_secs(300),
        caps: &caps,
        environment: &[],
    });
    // A plan-limit stop or an interrupt ends Scoring with no window. Any
    // other failure is this bug's alone: it records no commit, the window
    // keeps every other proposal, and its artifacts are published with them.
    match ran {
        Err(error) if error.is::<crate::session::limits::Reached>() => {
            anyhow::bail!("Scoring stopped at a plan-limit warning; window was not advanced")
        }
        Err(error) if interrupt::requested() => return Err(error),
        Err(error) => return Ok(failed(&format!("Bug-origin session failed: {error:#}"))),
        Ok(()) => {}
    }
    let proposal = workspace.join("proposal.json");
    let text = match private::read_session_file(&proposal, private::SESSION_OUTPUT_MAX) {
        Err(error) if private::is_missing_file(&error) => {
            return Ok(failed("Bug-origin session failed: it left no proposal"));
        }
        read => read?,
    };
    let response: Response = serde_json::from_slice(&text)?;
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

/// A proposal for a bug whose session failed: no commit, and where to look.
fn failed(cause: &str) -> Response {
    Response {
        commit: None,
        reason: format!(
            "{cause}. No introducing commit was established; the session's artifacts are in this window's packets/origins/, and the Day shift can settle the bug by hand."
        ),
    }
}

fn is_commit(commit: &str) -> bool {
    commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}
