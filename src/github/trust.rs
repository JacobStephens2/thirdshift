//! The GitHub adapter's reads for the Trusted account rule in
//! [`crate::trust`]: the queries, their paging and parsing, and the reading
//! of membership endpoints.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::{CommandFailure, GitHub};
use crate::issue::IssueUrl;
use crate::labels::READY_FOR_AGENT;
use crate::trust::{Action, Author, IssueText, Reads, trusted_association};

impl Reads for GitHub {
    fn author(&self, issue: &IssueUrl) -> Result<Author> {
        let json = self.gh_json(&[
            "api",
            &format!("repos/{}/issues/{}", issue.repo_slug(), issue.number),
        ])?;
        Ok(Author {
            association: json["author_association"]
                .as_str()
                .context("gh api output has no author_association")?
                .to_string(),
            login: json["user"]["login"].as_str().map(str::to_string),
        })
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
    /// Each edit is parsed only when the rule reaches it.
    fn edits(
        &self,
        issue: &IssueUrl,
        text: IssueText,
    ) -> Result<impl Iterator<Item = Result<Action>>> {
        let (query, field) = match text {
            IssueText::Body => (BODY_EDITS_QUERY, "editedAt"),
            IssueText::Title => (TITLE_EDITS_QUERY, "createdAt"),
        };
        let nodes = self.paged_nodes(issue, query, "edits", "edit history")?.1;
        Ok(nodes.into_iter().map(move |node| action(&node, field)))
    }

    /// Public read access alone does not make an actor a repository
    /// collaborator: this checks actual membership.
    fn is_member(&self, issue: &IssueUrl, login: &str) -> Result<bool> {
        self.trusted_login(&issue.repo(), login)
    }
}

impl GitHub {
    /// Author filters need membership checked even when no PR is waiting.
    pub(crate) fn trusted_login(
        &self,
        repository: &crate::issue::Repo,
        login: &str,
    ) -> Result<bool> {
        if login.eq_ignore_ascii_case(&repository.owner) {
            return Ok(true);
        }
        if self.membership(&format!(
            "repos/{}/collaborators/{login}",
            repository.slug()
        ))? {
            return Ok(true);
        }
        let repo = self.gh_json(&["api", &format!("repos/{}", repository.slug())])?;
        if repo["owner"]["type"] == "Organization"
            && self.membership(&format!("orgs/{}/members/{login}", repository.owner))?
        {
            return Ok(true);
        }
        self.is_self(Some(login))
    }

    /// Whether an author with `association` and `login` is a Trusted account,
    /// where a membership read for each author would cost too much: a trusted
    /// association, or the account thirdshift's own credentials act as, such
    /// as a GitHub App's `<app>[bot]`, whose association is not trusted.
    pub(crate) fn trusted_commenter(&self, association: &str, login: Option<&str>) -> Result<bool> {
        Ok(trusted_association(association) || self.is_self(login)?)
    }

    /// Whether `login` is the account thirdshift's credentials act as. Work
    /// thirdshift published itself, under a GitHub App, is its own.
    fn is_self(&self, login: Option<&str>) -> Result<bool> {
        let Some(login) = login else {
            return Ok(false);
        };
        Ok(same_account(login, &self.authenticated_login()?))
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

/// Whether two logins name one account. REST spells a GitHub App's bot
/// `<app>[bot]`, and GraphQL spells it `<app>`.
fn same_account(a: &str, b: &str) -> bool {
    fn bare(login: &str) -> &str {
        login.strip_suffix("[bot]").unwrap_or(login)
    }
    bare(&a.to_ascii_lowercase()).eq(bare(&b.to_ascii_lowercase()))
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

#[cfg(test)]
mod tests {
    use super::same_account;

    #[test]
    fn a_bot_is_the_same_account_with_or_without_its_rest_suffix() {
        assert!(same_account("thirdshift-fleet[bot]", "Thirdshift-Fleet"));
        assert!(same_account("thirdshift-fleet", "thirdshift-fleet[bot]"));
        assert!(same_account("runner", "RUNNER"));
        assert!(!same_account("thirdshift-fleet[bot]", "outsider"));
    }
}
