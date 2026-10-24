//! Keep the PR head separate from the later merge tree and bug fix.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::super::{Connection, GitHub, LinkedIssue, issue_branch};
use crate::issue::IssueUrl;
use crate::security::fixing::SECURITY_FIX;

pub(crate) struct ReviewedPullRequest {
    pub body: String,
    pub base: String,
    pub head: String,
    pub input: String,
    pub trial: crate::trial::Inputs,
    pub merged_at: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    body: String,
    head_ref_oid: String,
    base_ref_name: String,
    head_ref_name: String,
    merged_at: DateTime<Utc>,
    commits: Commits,
    closing_issues_references: Connection<Linked>,
}

#[derive(Deserialize)]
struct Linked {
    url: String,
    #[serde(flatten)]
    labels: LinkedIssue,
}

#[derive(Deserialize)]
struct Commits {
    nodes: Vec<CommitNode>,
}
#[derive(Deserialize)]
struct CommitNode {
    commit: Parents,
}
#[derive(Deserialize)]
struct Parents {
    parents: Oids,
}
#[derive(Deserialize)]
struct Oids {
    nodes: Vec<Oid>,
}
#[derive(Deserialize)]
struct Oid {
    oid: String,
}

impl GitHub {
    pub(crate) fn bug_origin_review(&self, pr: &IssueUrl) -> Result<ReviewedPullRequest> {
        let json = self.issue_query(pr, QUERY, None)?;
        let metadata: Metadata =
            serde_json::from_value(json["data"]["repository"]["pullRequest"].clone())
                .context("invalid Bug-origin reviewed pull request")?;
        ensure!(
            !metadata.closing_issues_references.page_info.has_next_page,
            "Bug-origin linked issues are incomplete"
        );
        for issue in &metadata.closing_issues_references.nodes {
            ensure!(
                !issue.labels.labels.page_info.has_next_page,
                "Bug-origin issue labels are incomplete"
            );
            ensure!(
                !issue
                    .labels
                    .labels
                    .nodes
                    .iter()
                    .any(|label| SECURITY_FIX.is_named(&label.name)),
                "Security fixes cannot become Bug-origin Eval tasks"
            );
        }
        let inferred = issue_branch(&pr.repo_slug(), &metadata.head_ref_name)?;
        if let Some(issue) = &inferred {
            ensure!(
                !self.issue_labels(issue)?.has(SECURITY_FIX),
                "Security fixes cannot become Bug-origin Eval tasks"
            );
        }
        let issue = metadata
            .closing_issues_references
            .nodes
            .first()
            .map(|issue| IssueUrl::parse(&issue.url))
            .transpose()?
            .or(inferred)
            .context("Bug-origin pull request has no originating issue")?;
        let text = self.scoring_issue_at(&issue, metadata.merged_at)?;
        let base = metadata
            .commits
            .nodes
            .first()
            .and_then(|node| node.commit.parents.nodes.first())
            .context("Bug-origin pull request has no base commit")?
            .oid
            .clone();
        Ok(ReviewedPullRequest {
            body: metadata.body,
            base,
            head: metadata.head_ref_oid,
            input: format!("# {}\n\n{}\n", text.title, text.body),
            trial: crate::trial::Inputs {
                issue_url: issue.url,
                base: metadata.base_ref_name,
                branch: metadata.head_ref_name,
                pr_url: format!("https://github.com/{}/pull/{}", pr.repo_slug(), pr.number),
            },
            merged_at: metadata.merged_at,
        })
    }
}

const QUERY: &str = r#"query BugOriginReview($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) { pullRequest(number: $number) {
    body headRefOid baseRefName headRefName mergedAt
    commits(first: 1) { nodes { commit { parents(first: 1) { nodes { oid } } } } }
    closingIssuesReferences(first: 100) { pageInfo { hasNextPage } nodes {
      url labels(first: 100) { pageInfo { hasNextPage } nodes { name } }
    } }
  } }
}"#;
