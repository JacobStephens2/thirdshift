//! Asking GitHub, through `gh`, about pull requests.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::issue::IssueUrl;

pub struct PullRequest {
    pub url: String,
    pub state: String,
    pub base: String,
    pub is_draft: bool,
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let output = Command::new("gh")
        .args(["pr", "view", branch, "--repo", &issue.repo_slug()])
        .args(["--json", "url,state,baseRefName,isDraft"])
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
        is_draft: json["isDraft"]
            .as_bool()
            .context("gh pr view output has no isDraft")?,
    }))
}

/// Mark the pull request whose head is `branch` ready for review, or, with
/// `draft`, convert it back to a draft.
pub fn set_draft(issue: &IssueUrl, branch: &str, draft: bool) -> Result<()> {
    let mut command = Command::new("gh");
    command.args(["pr", "ready", branch, "--repo", &issue.repo_slug()]);
    if draft {
        command.arg("--undo");
    }
    let output = command.output().context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh pr ready {branch} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
