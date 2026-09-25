//! Asking GitHub, through `gh`, about pull requests.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::issue::IssueUrl;

const PR_FIELDS: &str = "number,url,state,headRefName,baseRefName";

pub struct PullRequest {
    pub number: u64,
    pub url: String,
    /// `OPEN`, `CLOSED` or `MERGED`.
    pub state: String,
    pub head: String,
    pub base: String,
}

impl PullRequest {
    fn from_json(json: &Value) -> Result<Self> {
        let field = |name: &str| -> Result<String> {
            json[name]
                .as_str()
                .map(String::from)
                .with_context(|| format!("gh output has no {name}"))
        };
        Ok(PullRequest {
            number: json["number"].as_u64().context("gh output has no number")?,
            url: field("url")?,
            state: field("state")?,
            head: field("headRefName")?,
            base: field("baseRefName")?,
        })
    }
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let output = Command::new("gh")
        .args(["pr", "view", branch, "--repo", &issue.repo_slug()])
        .args(["--json", PR_FIELDS])
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no pull requests found") {
            return Ok(None);
        }
        bail!("gh pr view {branch} failed: {}", stderr.trim());
    }
    let json: Value =
        serde_json::from_slice(&output.stdout).context("gh pr view returned invalid JSON")?;
    PullRequest::from_json(&json).map(Some)
}

/// Every pull request in this repository, in any state, whose head branch
/// starts with `prefix`. PRs from forks are left out: their heads are not
/// this repository's branches.
pub fn pull_requests_from(issue: &IssueUrl, prefix: &str) -> Result<Vec<PullRequest>> {
    let output = Command::new("gh")
        .args(["pr", "list", "--repo", &issue.repo_slug(), "--state", "all"])
        .args(["--search", &format!("head:{prefix}"), "--limit", "1000"])
        .args(["--json", &format!("{PR_FIELDS},isCrossRepository")])
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh pr list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let json: Value =
        serde_json::from_slice(&output.stdout).context("gh pr list returned invalid JSON")?;
    json.as_array()
        .context("gh pr list did not return a list")?
        .iter()
        .filter(|pr| pr["isCrossRepository"] != Value::Bool(true))
        .map(PullRequest::from_json)
        .collect()
}
