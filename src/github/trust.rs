//! The GitHub adapter's reads for the Trusted account rule in
//! [`crate::trust`]: the queries, their paging and parsing, and the reading
//! of membership endpoints.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{CommandFailure, GitHub};
use crate::issue::IssueUrl;
use crate::labels::READY_FOR_AGENT;
use crate::trust::{Action, IssueText, Reads};

impl Reads for GitHub {
    fn author_association(&self, issue: &IssueUrl) -> Result<String> {
        let json = self.gh_json(&[
            "api",
            &format!("repos/{}/issues/{}", issue.repo_slug(), issue.number),
        ])?;
        Ok(json["author_association"]
            .as_str()
            .context("gh api output has no author_association")?
            .to_string())
    }

    /// A Claim removes the label, but its historical event still matters.
    /// Every page is read: a later reapplication supersedes an earlier one.
    fn ready_labellings(&self, issue: &IssueUrl) -> Result<Vec<Action>> {
        let events = self.gh_api_items(
            &format!(
                "repos/{}/issues/{}/events?per_page=100",
                issue.repo_slug(),
                issue.number
            ),
            "",
        )?;
        events
            .iter()
            .filter(|event| {
                event["event"] == "labeled"
                    && event["label"]["name"]
                        .as_str()
                        .is_some_and(|name| READY_FOR_AGENT.is_named(name))
            })
            .map(|event| action(event, "created_at"))
            .collect()
    }

    /// Its `edits` connection is read page by page so that no edit is missed.
    fn edits(&self, issue: &IssueUrl, text: IssueText) -> Result<Vec<Action>> {
        let (query, field) = match text {
            IssueText::Body => (BODY_EDITS_QUERY, "editedAt"),
            IssueText::Title => (TITLE_EDITS_QUERY, "createdAt"),
        };
        self.paged_nodes(issue, query, "edits", "edit history")?
            .1
            .iter()
            .map(|node| action(node, field))
            .collect()
    }

    /// Public read access alone does not make an actor a repository
    /// collaborator: this checks actual membership.
    fn is_member(&self, issue: &IssueUrl, login: &str) -> Result<bool> {
        if self.membership(&format!(
            "repos/{}/collaborators/{login}",
            issue.repo_slug()
        ))? {
            return Ok(true);
        }
        let repo = self.gh_json(&["api", &format!("repos/{}", issue.repo_slug())])?;
        if repo["owner"]["type"] == "Organization" {
            return self.membership(&format!("orgs/{}/members/{login}", issue.owner));
        }
        Ok(false)
    }
}

impl GitHub {
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

/// The account that made the event or edit `node`, if GitHub still knows it,
/// and when, from its `field`.
fn action(node: &Value, field: &str) -> Result<Action> {
    Ok(Action {
        actor: node["actor"]["login"].as_str().map(str::to_string),
        at: time_at(node, field)?,
    })
}

/// The time in `node`'s `field`, when an event or edit was made.
fn time_at(node: &Value, field: &str) -> Result<DateTime<Utc>> {
    let at = node[field].as_str().with_context(|| {
        CommandFailure::InvalidResponse(format!("issue history has an entry with no {field}"))
    })?;
    Ok(DateTime::parse_from_rfc3339(at)
        .with_context(|| {
            CommandFailure::InvalidResponse(format!("issue history has an unreadable {field} {at}"))
        })?
        .to_utc())
}

/// The edits to an issue's body, each with the account that made it as
/// `actor`. GitHub keeps the first revision here too, by its author.
const BODY_EDITS_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      edits: userContentEdits(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { editedAt actor: editor { login } }
      }
    }
  }
}";

/// The edits to an issue's title: its `RenamedTitleEvent`s.
const TITLE_EDITS_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      edits: timelineItems(first: 100, after: $after, itemTypes: [RENAMED_TITLE_EVENT]) {
        pageInfo { hasNextPage endCursor }
        nodes { ... on RenamedTitleEvent { createdAt actor { login } } }
      }
    }
  }
}";
