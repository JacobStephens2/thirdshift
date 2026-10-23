//! GitHub's merged factory PR search for the Scoring sample.
use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

use super::GitHub;
use crate::issue::IssueUrl;
use crate::scoring::PullRequest;
use crate::security::fixing::SECURITY_FIX;

mod packet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MergedPullRequest {
    number: u64,
    url: String,
    body: String,
    head_ref_name: String,
    merged_at: DateTime<Utc>,
    repository: Repository,
    closing_issues_references: Connection<LinkedIssue>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection<T> {
    nodes: Vec<T>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
}

#[derive(Deserialize)]
struct LinkedIssue {
    labels: Connection<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

const SEARCH: &str = r#"query ScoringPullRequests($search: String!, $after: String) {
  search(query: $search, type: ISSUE, first: 50, after: $after) {
    issueCount pageInfo { hasNextPage endCursor }
    nodes { ... on PullRequest {
      number url body mergedAt headRefName repository { nameWithOwner }
      closingIssuesReferences(first: 50) {
        pageInfo { hasNextPage }
        nodes { url labels(first: 100) { nodes { name } pageInfo { hasNextPage } } }
      }
    } }
  }
}"#;

impl GitHub {
    pub fn scoring_pull_requests(
        &self,
        owners: &[String],
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<PullRequest>> {
        let mut prs = Vec::new();
        for owner in owners {
            let search = format!(
                "is:pr is:merged user:{owner} in:body \"thirdshift:built-with\" merged:{}..{}",
                start.to_rfc3339_opts(SecondsFormat::Secs, true),
                end.to_rfc3339_opts(SecondsFormat::Secs, true)
            );
            let mut after = None;
            loop {
                let query_arg = format!("query={SEARCH}");
                let search_arg = format!("search={search}");
                let cursor_arg = format!("after={}", after.as_deref().unwrap_or(""));
                let mut args = vec!["api", "graphql", "-f", &query_arg, "-f", &search_arg];
                if after.is_some() {
                    args.extend(["-f", &cursor_arg]);
                }
                let value = self.gh_json(&args)?;
                if value.get("errors").is_some() {
                    bail!("Scoring search returned GraphQL errors");
                }
                let found = &value["data"]["search"];
                if found["issueCount"]
                    .as_u64()
                    .context("Scoring search has no issueCount")?
                    > 1000
                {
                    bail!(
                        "Scoring search exceeds GitHub's 1000-result limit; window was not advanced"
                    );
                }
                for value in found["nodes"]
                    .as_array()
                    .context("Scoring search has no nodes")?
                {
                    let pr: MergedPullRequest = serde_json::from_value(value.clone())
                        .context("invalid Scoring pull request")?;
                    if pr.merged_at < start || pr.merged_at >= end {
                        continue;
                    }
                    if pr.closing_issues_references.page_info.has_next_page
                        || pr
                            .closing_issues_references
                            .nodes
                            .iter()
                            .any(|issue| issue.labels.page_info.has_next_page)
                    {
                        bail!(
                            "Scoring cannot verify all linked issue labels for {}; window was not advanced",
                            pr.url
                        );
                    }
                    if pr.closing_issues_references.nodes.iter().any(|issue| {
                        issue
                            .labels
                            .nodes
                            .iter()
                            .any(|label| SECURITY_FIX.is_named(&label.name))
                    }) {
                        continue;
                    }
                    // GitHub does not infer closing links for every non-default
                    // Base branch. A factory Issue branch still names its issue.
                    if let Some(number) = pr
                        .head_ref_name
                        .strip_prefix("issue-")
                        .and_then(|rest| rest.split("-branch-").next())
                        .filter(|number| {
                            !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
                        })
                    {
                        let issue = IssueUrl::parse(&format!(
                            "https://github.com/{}/issues/{number}",
                            pr.repository.name_with_owner
                        ))?;
                        if self.issue_labels(&issue)?.has(SECURITY_FIX) {
                            continue;
                        }
                    }
                    if let Some(pr) = PullRequest::from_body(
                        pr.repository.name_with_owner,
                        pr.number,
                        pr.url,
                        pr.merged_at,
                        &pr.body,
                    ) {
                        prs.push(pr);
                    }
                }
                if !found["pageInfo"]["hasNextPage"]
                    .as_bool()
                    .context("Scoring search has no pageInfo")?
                {
                    break;
                }
                let cursor = found["pageInfo"]["endCursor"]
                    .as_str()
                    .context("Scoring search has no cursor")?;
                if after.as_deref() == Some(cursor) {
                    bail!("Scoring search cursor did not advance");
                }
                after = Some(cursor.to_string());
            }
        }
        Ok(prs)
    }
}
