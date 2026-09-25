//! Asking GitHub, through `gh`, about pull requests.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::issue::IssueUrl;

pub struct PullRequest {
    pub url: String,
    pub state: String,
    pub base: String,
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let output = Command::new("gh")
        .args(["pr", "view", branch, "--repo", &issue.repo_slug()])
        .args(["--json", "url,state,baseRefName"])
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
    let field = |name: &str| -> Result<String> {
        json[name]
            .as_str()
            .map(String::from)
            .with_context(|| format!("gh pr view output has no {name}"))
    };
    Ok(Some(PullRequest {
        url: field("url")?,
        state: field("state")?,
        base: field("baseRefName")?,
    }))
}
