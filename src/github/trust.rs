//! The fixed Trusted account rule, shared by every path that starts work.

use std::fmt;

use anyhow::{Context, Result, bail};

use super::GitHub;
use crate::issue::IssueUrl;
use crate::labels::READY_FOR_AGENT;

/// The first trust check an issue failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UntrustedIssue {
    Author,
    ReadyLabel,
}

impl fmt::Display for UntrustedIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Author => "issue author is not a Trusted account",
            Self::ReadyLabel => "most recent ready-for-agent label actor is not a Trusted account",
        })
    }
}

impl GitHub {
    /// Check the issue author and the most recent ready-for-agent label actor.
    pub fn issue_trust(&self, issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
        let json = self.gh_json(&[
            "api",
            &format!("repos/{}/issues/{}", issue.repo_slug(), issue.number),
        ])?;
        let association = json["author_association"]
            .as_str()
            .context("gh api output has no author_association")?;
        if !matches!(association, "OWNER" | "MEMBER" | "COLLABORATOR") {
            return Ok(Some(UntrustedIssue::Author));
        }
        // A Claim removes the label, but its historical event still matters.
        // Read every page: a later reapplication supersedes an earlier one.
        let events = self.gh_api_items(
            &format!(
                "repos/{}/issues/{}/events?per_page=100",
                issue.repo_slug(),
                issue.number
            ),
            "",
        )?;
        let actor = events
            .iter()
            .rfind(|event| {
                event["event"] == "labeled"
                    && event["label"]["name"]
                        .as_str()
                        .is_some_and(|name| READY_FOR_AGENT.is_named(name))
            })
            .and_then(|event| event["actor"]["login"].as_str());
        let Some(actor) = actor else {
            return Ok(Some(UntrustedIssue::ReadyLabel));
        };
        if !self.trusted_actor(issue, actor)? {
            return Ok(Some(UntrustedIssue::ReadyLabel));
        }
        Ok(None)
    }

    /// Events have no author association. Check actual membership: public
    /// read access alone does not make an actor a repository collaborator.
    fn trusted_actor(&self, issue: &IssueUrl, actor: &str) -> Result<bool> {
        if actor.eq_ignore_ascii_case(&issue.owner) {
            return Ok(true);
        }
        if self.membership(&format!(
            "repos/{}/collaborators/{actor}",
            issue.repo_slug()
        ))? {
            return Ok(true);
        }
        let repo = self.gh_json(&["api", &format!("repos/{}", issue.repo_slug())])?;
        if repo["owner"]["type"] == "Organization" {
            return self.membership(&format!("orgs/{}/members/{actor}", issue.owner));
        }
        Ok(false)
    }

    /// Membership endpoints return 204 for a member and 404 otherwise.
    /// Other failures must stop unattended selection instead of granting trust.
    fn membership(&self, path: &str) -> Result<bool> {
        let output = self.output(&["api", "--include", path])?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let status = stdout
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1));
        match status {
            Some("204") if output.status.success() => Ok(true),
            Some("404") => Ok(false),
            _ => bail!(
                "could not check Trusted account membership: gh api {path}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }
    }
}
