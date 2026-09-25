//! Asking GitHub, through `gh`, about issues and pull requests.

use std::fmt;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::issue::IssueUrl;

/// Whether `issue` is open.
pub fn issue_is_open(issue: &IssueUrl) -> Result<bool> {
    let json = gh_json(&[
        "issue",
        "view",
        &issue.number.to_string(),
        "--repo",
        &issue.repo_slug(),
        "--json",
        "state",
    ])?;
    Ok(json["state"].as_str().context("gh output has no state")? == "OPEN")
}

const PR_FIELDS: &str = "number,url,state,headRefName,baseRefName,isDraft";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl fmt::Display for PrState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
        })
    }
}

pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub head: String,
    pub base: String,
    pub is_draft: bool,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == PrState::Open
    }

    fn from_json(json: &Value) -> Result<Self> {
        let field = |name: &str| -> Result<String> {
            json[name]
                .as_str()
                .map(String::from)
                .with_context(|| format!("gh output has no {name}"))
        };
        let state = match field("state")?.as_str() {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            other => bail!("gh output has an unknown PR state {other}"),
        };
        Ok(PullRequest {
            number: json["number"].as_u64().context("gh output has no number")?,
            url: field("url")?,
            state,
            head: field("headRefName")?,
            base: field("baseRefName")?,
            is_draft: json["isDraft"]
                .as_bool()
                .context("gh output has no isDraft")?,
        })
    }
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let json = match gh_json(&[
        "pr",
        "view",
        branch,
        "--repo",
        &issue.repo_slug(),
        "--json",
        PR_FIELDS,
    ]) {
        Ok(json) => json,
        Err(error) if format!("{error:#}").contains("no pull requests found") => return Ok(None),
        Err(error) => return Err(error),
    };
    PullRequest::from_json(&json).map(Some)
}

/// Every pull request in this repository, in any state, whose head branch
/// starts with `prefix`. PRs from forks are left out: their heads are not
/// this repository's branches.
pub fn pull_requests_with_head_prefix(issue: &IssueUrl, prefix: &str) -> Result<Vec<PullRequest>> {
    let json = gh_json(&[
        "pr",
        "list",
        "--repo",
        &issue.repo_slug(),
        "--state",
        "all",
        "--search",
        &format!("head:{prefix}"),
        "--limit",
        "1000",
        "--json",
        &format!("{PR_FIELDS},isCrossRepository"),
    ])?;
    json.as_array()
        .context("gh pr list did not return a list")?
        .iter()
        .filter(|pr| pr["isCrossRepository"] != Value::Bool(true))
        .map(PullRequest::from_json)
        .collect()
}

/// Mark the pull request whose head is `branch` ready for review.
pub fn mark_ready(issue: &IssueUrl, branch: &str) -> Result<()> {
    gh(&["pr", "ready", branch, "--repo", &issue.repo_slug()])
}

/// Convert the pull request whose head is `branch` back to a draft.
pub fn convert_to_draft(issue: &IssueUrl, branch: &str) -> Result<()> {
    gh(&[
        "pr",
        "ready",
        branch,
        "--undo",
        "--repo",
        &issue.repo_slug(),
    ])
}

/// Run `gh <args>`, failing with its stderr if it exits non-zero.
fn gh(args: &[&str]) -> Result<()> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Run `gh <args>` and parse its stdout as JSON, failing with gh's stderr if
/// it exits non-zero.
fn gh_json(args: &[&str]) -> Result<Value> {
    let command = format!("gh {}", args[..2].join(" "));
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("{command} returned invalid JSON"))
}
