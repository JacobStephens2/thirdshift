//! Resolve a Command's GitHub target (issue, pull requests, and titles) for reader pages.

use std::collections::BTreeSet;

use serde_json::Value;

use super::super::command_record::Issue;
use super::super::github::Kept;
use super::super::reading::{Command, Observation};
use crate::issue::{IssueUrl, Repo};
use crate::review::PullRequestUrl;

pub(in crate::logs) const MAX_DISPATCH_DEPTH: usize = 8;

/// One validated GitHub issue or pull request link and its optional title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TargetLink {
    pub label: String,
    pub url: Option<String>,
    pub title: Option<String>,
}

/// The GitHub issue and/or pull requests worked on by a selected Command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubTarget {
    pub primary: Option<TargetLink>,
    pub pull_requests: Vec<TargetLink>,
}

impl GitHubTarget {
    /// Extract a target from a local snapshot `Observation` and optional `Kept` GitHub cache.
    pub(in crate::logs) fn from_observation(
        observation: Observation<'_>,
        kept: Option<&Kept>,
    ) -> Option<Self> {
        let selector = observation.selector();
        match observation {
            Observation::Record(command) => {
                let repository = command.record.repository.as_str();
                let primary = issue_target(command.record.issue.as_ref(), repository);
                let mut pull_requests = Vec::new();
                let mut seen = BTreeSet::new();
                collect_pr_links(
                    CommandPrSource { command, kept },
                    repository,
                    selector.as_deref(),
                    &mut pull_requests,
                    &mut seen,
                    0,
                );
                finalize(primary, pull_requests)
            }
            Observation::History(historical) => {
                let primary = issue_target(historical.issue.as_ref(), &historical.repository);
                finalize(primary, Vec::new())
            }
        }
    }

    /// Extract and validate a target from a remote peer's `/api/v1/peek` `command` JSON object.
    pub(crate) fn from_details(command: &Value) -> Option<Self> {
        let repository = command["repository"].as_str().unwrap_or("");
        let primary = issue_from_value(&command["issue"], repository);
        let subjects = &command["github"]["subjects"];
        let mut pull_requests = Vec::new();
        let mut seen = BTreeSet::new();
        collect_pr_links(
            DetailsPrSource {
                node: command,
                subjects,
            },
            repository,
            None,
            &mut pull_requests,
            &mut seen,
            0,
        );
        finalize(primary, pull_requests)
    }
}

fn non_empty(text: Option<&str>) -> Option<String> {
    text.filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

fn is_safe_github_url(raw: &str) -> bool {
    reqwest::Url::parse(raw).is_ok_and(|parsed| {
        parsed.scheme() == "https"
            && parsed.host_str() == Some("github.com")
            && parsed.port().is_none()
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed.query().is_none()
            && parsed.fragment().is_none()
    })
}

fn valid_repo_slug(repo: &str) -> bool {
    Repo::parse_slug(repo).is_some()
}

fn validated_issue_url(
    url: &str,
    repository: &str,
    expected_number: Option<u64>,
) -> Option<IssueUrl> {
    if !is_safe_github_url(url) {
        return None;
    }
    let parsed = IssueUrl::parse(url).ok()?;
    if parsed.number == 0 || expected_number.is_some_and(|number| parsed.number != number) {
        return None;
    }
    if !repository.is_empty() && !parsed.repo_slug().eq_ignore_ascii_case(repository) {
        return None;
    }
    Some(parsed)
}

fn validated_pr_url(url: &str, repository: &str) -> Option<PullRequestUrl> {
    if !is_safe_github_url(url) {
        return None;
    }
    let parsed = PullRequestUrl::parse(url).ok()?;
    if parsed.issue.number == 0 {
        return None;
    }
    if !repository.is_empty() && !parsed.issue.repo_slug().eq_ignore_ascii_case(repository) {
        return None;
    }
    Some(parsed)
}

fn issue_target(issue: Option<&Issue>, repository: &str) -> Option<TargetLink> {
    let issue = issue?;
    if issue.number == 0 {
        return None;
    }
    let title = non_empty(issue.title.as_deref());
    if let Some(parsed) = validated_issue_url(&issue.url, repository, Some(issue.number)) {
        return Some(TargetLink {
            label: format!("{}#{}", parsed.repo_slug(), parsed.number),
            url: Some(parsed.url),
            title,
        });
    }
    valid_repo_slug(repository).then(|| TargetLink {
        label: format!("{repository}#{}", issue.number),
        url: None,
        title,
    })
}

fn issue_from_value(issue: &Value, repository: &str) -> Option<TargetLink> {
    let object = issue.as_object()?;
    let number = object.get("number").and_then(Value::as_u64)?;
    let url = object.get("url").and_then(Value::as_str).unwrap_or("");
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let candidate = Issue {
        number,
        url: url.to_owned(),
        title,
    };
    issue_target(Some(&candidate), repository)
}

fn merge_pull_request(prs: &mut Vec<TargetLink>, seen: &mut BTreeSet<String>, link: TargetLink) {
    if let Some(url) = link.url.as_deref()
        && !seen.insert(url.to_owned())
    {
        if link.title.is_some()
            && let Some(existing) = prs
                .iter_mut()
                .find(|item| item.url.as_deref() == Some(url) && item.title.is_none())
        {
            existing.title = link.title;
        }
        return;
    }
    prs.push(link);
}

fn kept_pr_title(url: &str, rows: &[&str], kept: Option<&Kept>) -> Option<String> {
    let kept = kept?;
    for row in rows {
        if let Some(title) = kept
            .pull_request(row, url)
            .and_then(|state| non_empty(state.title.as_deref()))
        {
            return Some(title);
        }
    }
    kept.pull_request("", url)
        .and_then(|state| non_empty(state.title.as_deref()))
}

trait PrSource<'a>: Sized {
    fn repository(&self) -> Option<&'a str>;
    fn pull_request(&self) -> Option<&'a str>;
    fn pr_title(&self, parsed_url: &str, raw_url: &str, outer_row: Option<&str>) -> Option<String>;
    fn dispatched(&self) -> Vec<Self>;
}

struct CommandPrSource<'a> {
    command: &'a Command,
    kept: Option<&'a Kept>,
}

impl<'a> PrSource<'a> for CommandPrSource<'a> {
    fn repository(&self) -> Option<&'a str> {
        Some(self.command.record.repository.as_str())
    }

    fn pull_request(&self) -> Option<&'a str> {
        self.command.record.pull_request.as_deref()
    }

    fn pr_title(
        &self,
        parsed_url: &str,
        _raw_url: &str,
        outer_row: Option<&str>,
    ) -> Option<String> {
        let own_selector = Observation::Record(self.command).selector();
        let mut rows = Vec::new();
        if let Some(sel) = own_selector.as_deref() {
            rows.push(sel);
        }
        if let Some(sel) = outer_row
            && !rows.contains(&sel)
        {
            rows.push(sel);
        }
        kept_pr_title(parsed_url, &rows, self.kept)
            .or_else(|| non_empty(self.command.record.pull_request_title.as_deref()))
    }

    fn dispatched(&self) -> Vec<Self> {
        self.command
            .dispatched
            .iter()
            .map(|child| CommandPrSource {
                command: child,
                kept: self.kept,
            })
            .collect()
    }
}

struct DetailsPrSource<'a> {
    node: &'a Value,
    subjects: &'a Value,
}

impl<'a> PrSource<'a> for DetailsPrSource<'a> {
    fn repository(&self) -> Option<&'a str> {
        self.node["repository"].as_str()
    }

    fn pull_request(&self) -> Option<&'a str> {
        self.node["pull_request"].as_str()
    }

    fn pr_title(
        &self,
        parsed_url: &str,
        raw_url: &str,
        _outer_row: Option<&str>,
    ) -> Option<String> {
        non_empty(self.subjects[parsed_url]["facts"]["title"].as_str())
            .or_else(|| non_empty(self.subjects[raw_url]["facts"]["title"].as_str()))
            .or_else(|| non_empty(self.node["pull_request_title"].as_str()))
    }

    fn dispatched(&self) -> Vec<Self> {
        self.node["dispatched"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| DetailsPrSource {
                node: child,
                subjects: self.subjects,
            })
            .collect()
    }
}

fn collect_pr_links<'a, S: PrSource<'a>>(
    source: S,
    fallback_repo: &'a str,
    outer_row: Option<&str>,
    prs: &mut Vec<TargetLink>,
    seen: &mut BTreeSet<String>,
    depth: usize,
) {
    if depth > MAX_DISPATCH_DEPTH {
        return;
    }
    let repo = source
        .repository()
        .filter(|repo| !repo.is_empty())
        .unwrap_or(fallback_repo);
    if let Some(pr_url) = source.pull_request()
        && let Some(parsed) = validated_pr_url(pr_url, repo)
    {
        let title = source.pr_title(&parsed.url, pr_url, outer_row);
        merge_pull_request(
            prs,
            seen,
            TargetLink {
                label: format!("{}#{}", parsed.issue.repo_slug(), parsed.issue.number),
                url: Some(parsed.url),
                title,
            },
        );
    }
    for child in source.dispatched() {
        collect_pr_links(child, repo, outer_row, prs, seen, depth + 1);
    }
}

fn finalize(
    primary: Option<TargetLink>,
    mut pull_requests: Vec<TargetLink>,
) -> Option<GitHubTarget> {
    if let Some(issue_title) = primary.as_ref().and_then(|issue| issue.title.as_deref()) {
        for pr in &mut pull_requests {
            if pr.title.as_deref() == Some(issue_title) {
                pr.title = None;
            }
        }
    }
    if primary.is_none() && pull_requests.is_empty() {
        None
    } else {
        Some(GitHubTarget {
            primary,
            pull_requests,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn from_details_suppresses_identical_pr_title_and_keeps_distinct_pr_title() {
        let same = json!({
            "repository": "acme/widgets",
            "issue": {"number": 42, "url": "https://github.com/acme/widgets/issues/42", "title": "Fix race"},
            "pull_request": "https://github.com/acme/widgets/pull/100",
            "pull_request_title": "Fix race",
            "dispatched": []
        });
        let target = GitHubTarget::from_details(&same).unwrap();
        assert_eq!(
            target
                .primary
                .as_ref()
                .and_then(|link| link.title.as_deref()),
            Some("Fix race")
        );
        assert_eq!(target.pull_requests.len(), 1);
        assert_eq!(target.pull_requests[0].label, "acme/widgets#100");
        assert_eq!(target.pull_requests[0].title, None);

        let distinct = json!({
            "repository": "acme/widgets",
            "issue": {"number": 42, "url": "https://github.com/acme/widgets/issues/42", "title": "Fix race"},
            "pull_request": null,
            "dispatched": [{
                "repository": "acme/widgets",
                "pull_request": "https://github.com/acme/widgets/pull/101",
                "pull_request_title": "PR: fix race & add test",
                "dispatched": []
            }]
        });
        let target = GitHubTarget::from_details(&distinct).unwrap();
        assert_eq!(
            target.pull_requests[0].title.as_deref(),
            Some("PR: fix race & add test")
        );
    }

    #[test]
    fn from_details_prefers_kept_pr_title_over_recorded_title() {
        let details = json!({
            "repository": "acme/widgets",
            "issue": null,
            "pull_request": "https://github.com/acme/widgets/pull/229",
            "pull_request_title": "Old recorded title",
            "github": {
                "subjects": {
                    "https://github.com/acme/widgets/pull/229": {
                        "facts": {
                            "state": "Open",
                            "title": "Renamed on GitHub",
                            "merged_at": null,
                            "closed_at": null,
                            "ci": "unknown",
                            "mergeable": "unknown"
                        },
                        "answered_at": "2026-11-01T12:00:00Z"
                    }
                }
            },
            "dispatched": []
        });
        let target = GitHubTarget::from_details(&details).unwrap();
        assert!(target.primary.is_none());
        assert_eq!(target.pull_requests.len(), 1);
        assert_eq!(target.pull_requests[0].label, "acme/widgets#229");
        assert_eq!(
            target.pull_requests[0].title.as_deref(),
            Some("Renamed on GitHub")
        );
    }

    #[test]
    fn from_details_rejects_non_github_urls_cross_repo_prs_and_deep_nesting() {
        let spoofed = json!({
            "repository": "acme/widgets",
            "issue": {"number": 7, "url": "https://evil.example/acme/widgets/issues/7", "title": "  "},
            "pull_request": "https://github.com/acme/widgets/pull/8?tab=files#diff",
            "dispatched": [{
                "repository": "acme/widgets",
                "pull_request": "https://github.com/other/repo/pull/9",
                "dispatched": []
            }]
        });
        let target = GitHubTarget::from_details(&spoofed).unwrap();
        let primary = target.primary.unwrap();
        assert_eq!(primary.label, "acme/widgets#7");
        assert_eq!(primary.url, None);
        assert_eq!(primary.title, None);
        assert!(target.pull_requests.is_empty());

        let invalid_repo = json!({
            "repository": "not a slug",
            "issue": {"number": 7, "url": "javascript:alert(1)", "title": "x"}
        });
        assert!(GitHubTarget::from_details(&invalid_repo).is_none());

        let mut nested = json!({
            "repository": "acme/widgets",
            "pull_request": "https://github.com/acme/widgets/pull/99",
            "dispatched": []
        });
        for _ in 0..10 {
            nested = json!({
                "repository": "acme/widgets",
                "dispatched": [nested]
            });
        }
        assert!(GitHubTarget::from_details(&nested).is_none());
    }
}
