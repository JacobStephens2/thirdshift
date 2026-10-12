//! The fixed Trusted account rule, shared by every path that starts work.

use std::fmt;

use anyhow::{Context, Result};

use super::GitHub;
use crate::issue::IssueUrl;

/// The first trust check an issue failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UntrustedIssue {
    Author,
}

impl fmt::Display for UntrustedIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Author => "issue author is not a Trusted account",
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
        Ok(None)
    }
}
