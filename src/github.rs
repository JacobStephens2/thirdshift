//! Asking GitHub, through `gh`, about issues and pull requests.

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
    ])?
    .context("issue not found")?;
    Ok(string_field(&json, "state")? == "OPEN")
}

pub struct PullRequest {
    pub url: String,
    pub state: String,
    pub base: String,
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let json = gh_json(&[
        "pr",
        "view",
        branch,
        "--repo",
        &issue.repo_slug(),
        "--json",
        "url,state,baseRefName",
    ])?;
    let Some(json) = json else {
        return Ok(None);
    };
    Ok(Some(PullRequest {
        url: string_field(&json, "url")?,
        state: string_field(&json, "state")?,
        base: string_field(&json, "baseRefName")?,
    }))
}

/// Run `gh <args>` and parse its JSON output, or `None` if `gh` found no
/// pull request.
fn gh_json(args: &[&str]) -> Result<Option<Value>> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh")?;
    let command = format!("gh {}", args.join(" "));
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("no pull requests found") {
            return Ok(None);
        }
        bail!("{command} failed: {}", stderr.trim());
    }
    serde_json::from_slice(&output.stdout)
        .map(Some)
        .with_context(|| format!("{command} returned invalid JSON"))
}

fn string_field(json: &Value, name: &str) -> Result<String> {
    json[name]
        .as_str()
        .map(String::from)
        .with_context(|| format!("gh output has no {name}"))
}
