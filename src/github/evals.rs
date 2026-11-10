//! Read bounded merged-PR candidates and their closing Tickets, without comments.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

use super::GitHub;
use crate::issue::Repo;

pub(crate) struct Candidate {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub base_ref_name: String,
    pub head_ref_name: String,
    pub merge: String,
    pub original: Option<OriginalChange>,
    tickets: std::result::Result<Vec<Ticket>, String>,
}

impl Candidate {
    pub fn tickets(&self) -> Result<&[Ticket]> {
        self.tickets
            .as_deref()
            .map_err(|reason| anyhow::anyhow!("{reason}"))
    }
}

pub(crate) struct OriginalChange {
    pub head: String,
    pub base: String,
    pub commits: usize,
}

pub(crate) struct Ticket {
    pub url: String,
    pub title: String,
    pub body: String,
    pub kind: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    number: u64,
    url: String,
    #[serde(default)]
    title: String,
    base_ref_name: String,
    head_ref_name: String,
    merge_commit: Commit,
    #[serde(default)]
    head_ref_oid: Option<String>,
    #[serde(default)]
    commits: Option<Commits>,
    closing_issues_references: Issues,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commits {
    total_count: usize,
    nodes: Vec<CommitNode>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: ParentCommit,
}

#[derive(Deserialize)]
struct ParentCommit {
    parents: Parents,
}

#[derive(Deserialize)]
struct Parents {
    nodes: Vec<Commit>,
}

impl Metadata {
    fn candidate(self) -> Candidate {
        let tickets = (|| -> Result<Vec<Ticket>> {
            ensure!(
                !self.closing_issues_references.page_info.has_next_page,
                "closing issues are incomplete"
            );
            let mut tickets = Vec::new();
            for issue in self.closing_issues_references.nodes {
                ensure!(
                    !issue.labels.page_info.has_next_page,
                    "Ticket labels are incomplete"
                );
                ensure!(
                    !issue
                        .labels
                        .nodes
                        .iter()
                        .any(|label| label.name.eq_ignore_ascii_case("security-fix")),
                    "Security fixes cannot become process Eval tasks"
                );
                if issue.sub_issues.total_count == 0 {
                    let kind = if issue
                        .labels
                        .nodes
                        .iter()
                        .any(|label| label.name.eq_ignore_ascii_case("bug"))
                    {
                        "bug"
                    } else {
                        "feature"
                    };
                    tickets.push(Ticket {
                        url: issue.url,
                        title: issue.title,
                        body: issue.body,
                        kind,
                    });
                }
            }
            ensure!(!tickets.is_empty(), "pull request closes no Ticket");
            Ok(tickets)
        })()
        .map_err(|error| error.to_string());
        let original = self
            .head_ref_oid
            .zip(self.commits)
            .and_then(|(head, commits)| {
                let base = commits
                    .nodes
                    .first()?
                    .commit
                    .parents
                    .nodes
                    .first()?
                    .oid
                    .clone();
                Some(OriginalChange {
                    head,
                    base,
                    commits: commits.total_count,
                })
            });
        Candidate {
            number: self.number,
            url: self.url,
            title: self.title,
            base_ref_name: self.base_ref_name,
            head_ref_name: self.head_ref_name,
            merge: self.merge_commit.oid,
            original,
            tickets,
        }
    }
}

#[derive(Deserialize)]
struct Commit {
    oid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Issues {
    nodes: Vec<Issue>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Issue {
    url: String,
    title: String,
    body: String,
    labels: Labels,
    sub_issues: Count,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Labels {
    nodes: Vec<Label>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
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
        let records: Vec<Metadata> =
            serde_json::from_value(json["data"]["repository"]["pullRequests"]["nodes"].clone())
                .context("invalid process candidates")?;
        Ok(records.into_iter().map(Metadata::candidate).collect())
    }
}

const QUERY: &str = r#"query ProcessCandidates($owner: String!, $repo: String!, $limit: Int!) {
  repository(owner: $owner, name: $repo) {
    pullRequests(first: $limit, states: MERGED, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number url title baseRefName headRefName headRefOid mergeCommit { oid }
        commits(first: 1) { totalCount nodes { commit { parents(first: 1) { nodes { oid } } } } }
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
