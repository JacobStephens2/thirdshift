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

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == "OPEN"
    }
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let output = gh(issue, &["pr", "view", branch])
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

/// Mark the pull request whose head is `branch` ready for review.
pub fn mark_ready(issue: &IssueUrl, branch: &str) -> Result<()> {
    run(issue, &["pr", "ready", branch])
}

/// Convert the pull request whose head is `branch` back to a draft.
pub fn convert_to_draft(issue: &IssueUrl, branch: &str) -> Result<()> {
    run(issue, &["pr", "ready", branch, "--undo"])
}

/// `gh <args> --repo <owner>/<repo>`, ready to run.
fn gh(issue: &IssueUrl, args: &[&str]) -> Command {
    let mut command = Command::new("gh");
    command.args(args).args(["--repo", &issue.repo_slug()]);
    command
}

/// Run `gh <args>`, failing with its stderr if it exits non-zero.
fn run(issue: &IssueUrl, args: &[&str]) -> Result<()> {
    let output = gh(issue, args).output().context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
