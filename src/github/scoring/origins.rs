//! Closed bugs are independent of the factory PR sample and who fixed them.
use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

use super::{Connection, GitHub, Label, Repository};
use crate::issue::IssueUrl;
use crate::security::fixing::SECURITY_FIX;

mod review;
pub(crate) use review::ReviewedPullRequest;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClosedBug {
    pub url: String,
    pub title: String,
    pub body: String,
    pub closed_at: DateTime<Utc>,
    repository: Repository,
    labels: Connection<Label>,
    timeline_items: Closures,
}

#[derive(Deserialize)]
struct Closures {
    nodes: Vec<Closure>,
}

#[derive(Deserialize)]
struct Closure {
    closer: Option<Closer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Closer {
    oid: Option<String>,
    merge_commit: Option<Commit>,
}

#[derive(Deserialize)]
struct Commit {
    oid: String,
}

impl ClosedBug {
    pub fn repository(&self) -> Result<String> {
        let issue = IssueUrl::parse(&self.url)?;
        anyhow::ensure!(
            issue
                .repo_slug()
                .eq_ignore_ascii_case(&self.repository.name_with_owner),
            "closed bug belongs to a different repository"
        );
        Ok(issue.repo_slug())
    }

    pub fn fix(&self) -> Option<&str> {
        let closer = self.timeline_items.nodes.last()?.closer.as_ref()?;
        closer.oid.as_deref().or_else(|| {
            closer
                .merge_commit
                .as_ref()
                .map(|commit| commit.oid.as_str())
        })
    }
}

#[derive(Deserialize)]
pub(crate) struct OriginPullRequest {
    pub html_url: String,
    pub body: Option<String>,
    pub merged_at: Option<DateTime<Utc>>,
}

impl GitHub {
    pub(crate) fn closed_bugs(
        &self,
        owners: &[String],
        start: Option<DateTime<Utc>>,
        end: DateTime<Utc>,
    ) -> Result<Vec<ClosedBug>> {
        let mut bugs = Vec::new();
        let mut seen = BTreeSet::new();
        for owner in owners {
            let mut windows = vec![(start, end)];
            while let Some((start, end)) = windows.pop() {
                let end_text = end.to_rfc3339_opts(SecondsFormat::Secs, true);
                let range = start.map_or_else(
                    // Search only understands whole seconds. Include the
                    // cutoff's second, then filter against the exact end.
                    || format!("<={end_text}"),
                    |start| {
                        format!(
                            "{}..{end_text}",
                            start.to_rfc3339_opts(SecondsFormat::Secs, true)
                        )
                    },
                );
                let search = format!(
                    "is:issue is:closed user:{owner} label:bug -label:security-fix closed:{range}"
                );
                let mut after = None;
                loop {
                    let mut args = vec![
                        "api".into(),
                        "graphql".into(),
                        "-f".into(),
                        format!("query={SEARCH}"),
                        "-f".into(),
                        format!("search={search}"),
                    ];
                    if let Some(cursor) = &after {
                        args.extend(["-f".into(), format!("after={cursor}")]);
                    }
                    let json =
                        self.gh_json(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
                    if json.get("errors").is_some() {
                        bail!("closed bug search returned GraphQL errors");
                    }
                    let found = &json["data"]["search"];
                    if found["issueCount"]
                        .as_u64()
                        .context("closed bug search has no issueCount")?
                        > 1000
                    {
                        // Search exposes at most 1,000 matches. Split time rather
                        // than silently truncating a first-run backfill. GitHub
                        // has no issues before the Unix epoch.
                        let lower = start.unwrap_or(DateTime::UNIX_EPOCH);
                        let seconds = (end - lower).num_seconds();
                        if seconds <= 1 {
                            bail!(
                                "more than 1000 bugs closed in one second; window was not advanced"
                            );
                        }
                        let midpoint = lower + chrono::TimeDelta::seconds(seconds / 2);
                        windows.extend([(Some(midpoint), end), (start, midpoint)]);
                        break;
                    }
                    for node in found["nodes"]
                        .as_array()
                        .context("closed bug search has no nodes")?
                    {
                        let bug: ClosedBug =
                            serde_json::from_value(node.clone()).context("invalid closed bug")?;
                        if bug.closed_at >= end || start.is_some_and(|start| bug.closed_at < start)
                        {
                            continue;
                        }
                        if bug.labels.page_info.has_next_page {
                            bail!("cannot verify all closed bug labels; window was not advanced");
                        }
                        if bug
                            .labels
                            .nodes
                            .iter()
                            .any(|label| SECURITY_FIX.is_named(&label.name))
                            || !bug
                                .labels
                                .nodes
                                .iter()
                                .any(|label| label.name.eq_ignore_ascii_case("bug"))
                        {
                            continue;
                        }
                        if seen.insert(bug.url.to_ascii_lowercase()) {
                            bugs.push(bug);
                        }
                    }
                    if !found["pageInfo"]["hasNextPage"]
                        .as_bool()
                        .context("closed bug search has no pageInfo")?
                    {
                        break;
                    }
                    let cursor = found["pageInfo"]["endCursor"]
                        .as_str()
                        .context("closed bug search has no cursor")?;
                    if after.as_deref() == Some(cursor) {
                        bail!("closed bug search cursor did not advance");
                    }
                    after = Some(cursor.to_string());
                }
            }
        }
        Ok(bugs)
    }

    pub(crate) fn origin_pull_requests(
        &self,
        repository: &str,
        commit: &str,
    ) -> Result<Vec<OriginPullRequest>> {
        self.gh_api_items(
            &format!("repos/{repository}/commits/{commit}/pulls?per_page=100"),
            "",
        )?
        .into_iter()
        .map(|value| serde_json::from_value(value).context("invalid Bug-origin pull request"))
        .collect()
    }
}

const SEARCH: &str = r#"query ClosedBugs($search: String!, $after: String) {
  search(query: $search, type: ISSUE, first: 50, after: $after) {
    issueCount pageInfo { hasNextPage endCursor }
    nodes { ... on Issue {
      url title body closedAt repository { nameWithOwner }
      labels(first: 100) { nodes { name } pageInfo { hasNextPage } }
      timelineItems(last: 1, itemTypes: [CLOSED_EVENT]) {
        nodes { ... on ClosedEvent { closer {
          ... on Commit { oid }
          ... on PullRequest { mergeCommit { oid } }
        } } }
      }
    } }
  }
}"#;
