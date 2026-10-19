//! The fixed Trusted account rule, shared by every path that starts work.

use std::fmt;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{CommandFailure, GitHub};
use crate::issue::IssueUrl;
use crate::labels::READY_FOR_AGENT;

/// The first trust check an issue failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UntrustedIssue {
    Author,
    ReadyLabel,
    /// Its `IssueText` was edited, after it was last marked ready, by an account
    /// that is not a Trusted account.
    Edited(IssueText),
}

impl fmt::Display for UntrustedIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Author => f.write_str("issue author is not a Trusted account"),
            Self::ReadyLabel => {
                f.write_str("most recent ready-for-agent label actor is not a Trusted account")
            }
            Self::Edited(edit) => write!(
                f,
                "{edit} edited after it was marked ready by an account that is not a Trusted account"
            ),
        }
    }
}

/// The part of an issue an edit changed: its body or its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueText {
    Body,
    Title,
}

impl fmt::Display for IssueText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Body => "body",
            Self::Title => "title",
        })
    }
}

/// One `ready-for-agent` labelling from an issue's event history.
struct Labelling {
    actor: Option<String>,
    at: DateTime<Utc>,
}

impl GitHub {
    /// Check the issue author, the most recent ready-for-agent label actor,
    /// and every edit to its body or title made since that labelling.
    pub fn issue_trust(&self, issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
        if let Some(check) = self.author_trust(issue)? {
            return Ok(Some(check));
        }
        let Some(Labelling {
            actor: Some(actor),
            at,
        }) = self.ready_labellings(issue)?.pop()
        else {
            return Ok(Some(UntrustedIssue::ReadyLabel));
        };
        if !self.trusted_actor(issue, &actor)? {
            return Ok(Some(UntrustedIssue::ReadyLabel));
        }
        Ok(self
            .untrusted_edit(issue, Some(at))?
            .map(UntrustedIssue::Edited))
    }

    /// Check a Ticket before its Run starts: its author, and every edit to
    /// its body or title made since the later of the Ticket's and its
    /// `spec`'s last ready-for-agent labellings by a Trusted account, or
    /// every edit if neither has one. A Ticket needs no label of its own:
    /// its Spec's labelling already passed the full check.
    pub fn ticket_trust(
        &self,
        ticket: &IssueUrl,
        spec: &IssueUrl,
    ) -> Result<Option<UntrustedIssue>> {
        if let Some(check) = self.author_trust(ticket)? {
            return Ok(Some(check));
        }
        let since = self
            .trusted_ready_at(ticket)?
            .max(self.trusted_ready_at(spec)?);
        Ok(self
            .untrusted_edit(ticket, since)?
            .map(UntrustedIssue::Edited))
    }

    /// The first edit to `issue`'s body or title made after `since`, or ever
    /// if `since` is `None`, by an account that is not a Trusted account, if
    /// there is one. An edit whose account GitHub no longer knows counts.
    pub fn untrusted_edit(
        &self,
        issue: &IssueUrl,
        since: Option<DateTime<Utc>>,
    ) -> Result<Option<IssueText>> {
        let mut trusted: Vec<String> = Vec::new();
        for (edit, query, field) in [
            (IssueText::Body, BODY_EDITS_QUERY, "editedAt"),
            (IssueText::Title, TITLE_EDITS_QUERY, "createdAt"),
        ] {
            for node in self.edits(issue, query)? {
                if let Some(since) = since
                    && time_at(&node, field)? <= since
                {
                    continue;
                }
                let Some(login) = node["actor"]["login"].as_str() else {
                    return Ok(Some(edit));
                };
                if trusted
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(login))
                {
                    continue;
                }
                if !self.trusted_actor(issue, login)? {
                    return Ok(Some(edit));
                }
                trusted.push(login.to_string());
            }
        }
        Ok(None)
    }

    /// Every edit `query` reads of `issue`, its `edits` connection read page
    /// by page so that no edit is missed.
    fn edits(&self, issue: &IssueUrl, query: &str) -> Result<Vec<Value>> {
        Ok(self.paged_nodes(issue, query, "edits", "edit history")?.1)
    }

    /// When `issue` was last labelled ready-for-agent by a Trusted account,
    /// if it ever was.
    fn trusted_ready_at(&self, issue: &IssueUrl) -> Result<Option<DateTime<Utc>>> {
        for labelling in self.ready_labellings(issue)?.into_iter().rev() {
            if let Some(actor) = &labelling.actor
                && self.trusted_actor(issue, actor)?
            {
                return Ok(Some(labelling.at));
            }
        }
        Ok(None)
    }

    /// Every ready-for-agent labelling of `issue`, oldest first. A Claim
    /// removes the label, but its historical event still matters. Every page
    /// is read: a later reapplication supersedes an earlier one.
    fn ready_labellings(&self, issue: &IssueUrl) -> Result<Vec<Labelling>> {
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
            .map(|event| {
                Ok(Labelling {
                    actor: event["actor"]["login"].as_str().map(str::to_string),
                    at: time_at(event, "created_at")?,
                })
            })
            .collect()
    }

    /// Check only the issue author.
    fn author_trust(&self, issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
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
