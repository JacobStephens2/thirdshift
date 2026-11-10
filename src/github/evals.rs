//! Read bounded merged-PR candidates and their closing Tickets, without comments.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::GitHub;
use crate::issue::Repo;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Candidate {
    pub number: u64,
    pub url: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub merge_commit: Commit,
    pub closing_issues_references: Issues,
}

#[derive(Deserialize)]
pub(crate) struct Commit {
    pub oid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Issues {
    pub nodes: Vec<Ticket>,
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ticket {
    pub url: String,
    pub title: String,
    pub body: String,
    pub labels: Labels,
    pub sub_issues: Count,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Count {
    pub total_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Labels {
    pub nodes: Vec<Label>,
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
pub(crate) struct Label {
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageInfo {
    pub has_next_page: bool,
}

impl GitHub {
    pub(crate) fn process_candidates(&self, repo: &Repo, limit: u32) -> Result<Vec<Candidate>> {
        let json = self.gh_json(&[
            "api",
            "graphql",
            "-f",
            &format!("query={QUERY}"),
            "-f",
            &format!("owner={}", repo.owner),
            "-f",
            &format!("repo={}", repo.name),
            "-F",
            &format!("limit={limit}"),
        ])?;
        ensure!(
            json.get("errors").is_none(),
            "process candidates returned GraphQL errors"
        );
        serde_json::from_value(json["data"]["repository"]["pullRequests"]["nodes"].clone())
            .context("invalid process candidates")
    }
}

const QUERY: &str = r#"query ProcessCandidates($owner: String!, $repo: String!, $limit: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequests(first: $limit, states: MERGED, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number url baseRefName headRefName mergeCommit { oid }
        closingIssuesReferences(first: 100) {
          pageInfo { hasNextPage }
          nodes {
            url title body subIssues(first: 1) { totalCount }
            labels(first: 100) { nodes { name } pageInfo { hasNextPage } }
          }
        }
      }
    }
  }
}"#;
