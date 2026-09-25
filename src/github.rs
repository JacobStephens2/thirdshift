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

/// Whether a pull request can be merged into its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    /// It conflicts with its base.
    No,
    /// GitHub is still working it out.
    Unknown,
}

/// Whether the pull request whose head is `branch` can be merged.
pub fn mergeable(issue: &IssueUrl, branch: &str) -> Result<Mergeable> {
    let json = gh_json(&[
        "pr",
        "view",
        branch,
        "--repo",
        &issue.repo_slug(),
        "--json",
        "mergeable",
    ])?;
    match json["mergeable"].as_str() {
        Some("MERGEABLE") => Ok(Mergeable::Yes),
        Some("CONFLICTING") => Ok(Mergeable::No),
        Some("UNKNOWN") => Ok(Mergeable::Unknown),
        other => bail!("gh output has an unknown mergeable state {other:?}"),
    }
}

/// Where a check or status stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Pending,
    Passed,
    Failed,
}

/// A check run or a commit status.
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub url: Option<String>,
}

/// Every check run and commit status on commit `sha`.
pub fn checks_on(issue: &IssueUrl, sha: &str) -> Result<Vec<Check>> {
    let commit = format!("repos/{}/commits/{sha}", issue.repo_slug());
    let runs = gh_api_items(&format!("{commit}/check-runs?per_page=100"), "check_runs")?;
    let statuses = gh_api_items(&format!("{commit}/status?per_page=100"), "statuses")?;
    let url = |value: &Value| {
        value
            .as_str()
            .filter(|url| !url.is_empty())
            .map(String::from)
    };

    let mut checks = Vec::new();
    for run in &runs {
        let state = match (run["status"].as_str(), run["conclusion"].as_str()) {
            (Some("completed"), Some("success" | "neutral" | "skipped")) => CheckState::Passed,
            (Some("completed"), _) => CheckState::Failed,
            _ => CheckState::Pending,
        };
        checks.push(Check {
            name: run["name"]
                .as_str()
                .context("a check run has no name")?
                .to_string(),
            state,
            url: url(&run["details_url"]).or_else(|| url(&run["html_url"])),
        });
    }
    for status in &statuses {
        let state = match status["state"].as_str() {
            Some("success") => CheckState::Passed,
            Some("pending") => CheckState::Pending,
            _ => CheckState::Failed,
        };
        checks.push(Check {
            name: status["context"]
                .as_str()
                .context("a commit status has no context")?
                .to_string(),
            state,
            url: url(&status["target_url"]),
        });
    }
    Ok(checks)
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

/// Every item of the `field` list of the GitHub API's `path`, across all its
/// pages.
fn gh_api_items(path: &str, field: &str) -> Result<Vec<Value>> {
    let output = Command::new("gh")
        .args(["api", "--paginate", "--jq", &format!(".{field}[]"), path])
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh api {path} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .with_context(|| format!("gh api {path} returned invalid JSON"))
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
