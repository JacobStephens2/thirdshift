//! Historical issue text and the minimal GitHub data for a blind packet.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Connection, GitHub, PullRequest, issue_branch};
use crate::issue::IssueUrl;
use crate::scoring::packet::{Check, Text};

pub(crate) struct PacketInput {
    pub merge_commit: String,
    pub base_commit: String,
    pub issues: Vec<Text>,
    pub pull_request: Text,
    pub ci: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    title: String,
    body: String,
    merge_commit: Commit,
    base_ref_oid: String,
    head_ref_oid: String,
    head_ref_name: String,
    closing_issues_references: Connection<LinkedIssue>,
}

#[derive(Deserialize)]
struct Commit {
    oid: String,
}

#[derive(Deserialize)]
struct LinkedIssue {
    url: String,
}

impl GitHub {
    pub(crate) fn scoring_packet(&self, pr: &PullRequest) -> Result<PacketInput> {
        let identity = IssueUrl::parse(&format!(
            "https://github.com/{}/issues/{}",
            pr.repository, pr.number
        ))?;
        let json = self.issue_query(&identity, METADATA, None)?;
        let metadata: Metadata =
            serde_json::from_value(json["data"]["repository"]["pullRequest"].clone())
                .context("invalid Scoring packet metadata")?;
        if metadata.closing_issues_references.page_info.has_next_page {
            bail!("Scoring packet cannot read all linked issues");
        }
        let mut linked: Vec<_> = metadata
            .closing_issues_references
            .nodes
            .into_iter()
            .map(|issue| IssueUrl::parse(&issue.url))
            .collect::<Result<_>>()?;
        if let Some(issue) = issue_branch(&pr.repository, &metadata.head_ref_name)?
            && !linked
                .iter()
                .any(|other| other.identity() == issue.identity())
        {
            linked.push(issue);
        }
        if linked.is_empty() {
            bail!("Scoring packet has no originating issue");
        }
        let issues = linked
            .iter()
            .map(|issue| self.scoring_issue_at(issue, pr.merged_at))
            .collect::<Result<_>>()?;
        let commit = format!("repos/{}/commits/{}", pr.repository, metadata.head_ref_oid);
        let mut ci = Vec::new();
        let checks = self.retry_read("Scoring CI checks", || {
            self.check_runs_on(&identity, &metadata.head_ref_oid)
        })?;
        for check in checks.records {
            ci.push(Check {
                name: text(&check, "name")?.into(),
                conclusion: check["conclusion"].as_str().map(str::to_owned),
            });
        }
        for status in self.gh_api_items(&format!("{commit}/status?per_page=100"), "statuses")? {
            ci.push(Check {
                name: text(&status, "context")?.into(),
                conclusion: Some(text(&status, "state")?.into()),
            });
        }
        Ok(PacketInput {
            merge_commit: metadata.merge_commit.oid,
            base_commit: metadata.base_ref_oid,
            issues,
            pull_request: Text {
                title: metadata.title,
                body: metadata.body,
            },
            ci,
        })
    }

    pub(in crate::github) fn scoring_issue_at(
        &self,
        issue: &IssueUrl,
        at: DateTime<Utc>,
    ) -> Result<Text> {
        let (json, edits) = self.paged_nodes(issue, BODY, "edits", "Scoring body history")?;
        let mut revisions = edits
            .iter()
            .map(|edit| Ok((time(edit, "editedAt")?, edit)))
            .collect::<Result<Vec<_>>>()?;
        revisions.sort_by_key(|(edited_at, _)| *edited_at);
        let node = &json["data"]["repository"]["issue"];
        let body = if let Some((_, edit)) = revisions
            .iter()
            .rev()
            .find(|(edited_at, _)| *edited_at <= at)
        {
            if !edit["deletedAt"].is_null() {
                bail!("Scoring issue revision was deleted: {}", issue.url);
            }
            // GitHub's UserContentEdit.diff contains the complete body revision.
            text(edit, "diff")?.to_string()
        } else if revisions.is_empty()
            && (node["lastEditedAt"].is_null() || time(node, "lastEditedAt")? <= at)
        {
            text(node, "body")?.to_string()
        } else {
            bail!("Scoring issue has no body revision at merge: {}", issue.url);
        };
        let (json, edits) = self.paged_nodes(issue, TITLE, "edits", "Scoring title history")?;
        let mut renames = edits
            .iter()
            .map(|edit| Ok((time(edit, "createdAt")?, edit)))
            .collect::<Result<Vec<_>>>()?;
        renames.sort_by_key(|(created_at, _)| *created_at);
        let title = if let Some((_, edit)) = renames.iter().find(|(created_at, _)| *created_at > at)
        {
            text(edit, "previousTitle")?
        } else {
            text(&json["data"]["repository"]["issue"], "title")?
        };
        Ok(Text {
            title: title.to_string(),
            body,
        })
    }
}

fn text<'a>(value: &'a serde_json::Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .with_context(|| format!("Scoring packet has no {field}"))
}

fn time(value: &serde_json::Value, field: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(text(value, field)?)?.to_utc())
}

const METADATA: &str = r#"query ScoringPacket($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      title body baseRefOid headRefName headRefOid mergeCommit { oid }
      closingIssuesReferences(first: 100) { pageInfo { hasNextPage } nodes { url } }
    }
  }
}"#;

const BODY: &str = r#"query ScoringBody($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      body lastEditedAt
      edits: userContentEdits(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor } nodes { editedAt deletedAt diff }
      }
    }
  }
}"#;

const TITLE: &str = r#"query ScoringTitle($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      title
      edits: timelineItems(first: 100, after: $after, itemTypes: [RENAMED_TITLE_EVENT]) {
        pageInfo { hasNextPage endCursor }
        nodes { ... on RenamedTitleEvent { createdAt previousTitle currentTitle } }
      }
    }
  }
}"#;
