//! Asking GitHub, through `gh`, about issues, pull requests and the signed-in
//! user.

use std::fmt;
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::issue::IssueUrl;

/// The `fields` of `issue`, as JSON.
fn issue_view(issue: &IssueUrl, fields: &str) -> Result<Value> {
    gh_json(&[
        "issue",
        "view",
        &issue.number.to_string(),
        "--repo",
        &issue.repo_slug(),
        "--json",
        fields,
    ])
}

/// Whether the issue `json` describes, with its `state`, is open.
fn state_is_open(json: &Value) -> Result<bool> {
    Ok(json["state"].as_str().context("gh output has no state")? == "OPEN")
}

/// Whether `issue` is open.
pub fn issue_is_open(issue: &IssueUrl) -> Result<bool> {
    state_is_open(&issue_view(issue, "state")?)
}

/// The title of `issue`.
pub fn issue_title(issue: &IssueUrl) -> Result<String> {
    let json = issue_view(issue, "title")?;
    Ok(json["title"]
        .as_str()
        .context("gh output has no title")?
        .to_string())
}

/// An issue as an Architect run reads the plan its Architecture review
/// published, and as a Claim reads its issue after a Self-merge.
pub struct Issue {
    pub is_open: bool,
    pub labels: Vec<String>,
    pub created: DateTime<Utc>,
}

/// `issue`'s state, labels and when it was created.
pub fn issue(issue: &IssueUrl) -> Result<Issue> {
    let json = issue_view(issue, "state,labels,createdAt")?;
    let created = json["createdAt"]
        .as_str()
        .context("gh output has no createdAt")?;
    Ok(Issue {
        is_open: state_is_open(&json)?,
        labels: label_names(&json)?,
        created: DateTime::parse_from_rfc3339(created)
            .with_context(|| format!("gh output has an unreadable createdAt {created}"))?
            .to_utc(),
    })
}

/// The names of the labels of the issue `json` describes, with its `labels`.
fn label_names(json: &Value) -> Result<Vec<String>> {
    Ok(json["labels"]
        .as_array()
        .context("gh output has no labels")?
        .iter()
        .filter_map(|label| label["name"].as_str().map(String::from))
        .collect())
}

/// Whether `label` is one of `labels`, whatever its case: GitHub's label
/// names are case-insensitive.
pub fn has_label(labels: &[String], label: &str) -> bool {
    labels.iter().any(|name| name.eq_ignore_ascii_case(label))
}

/// The labels of `issue`.
pub fn issue_labels(issue: &IssueUrl) -> Result<Vec<String>> {
    label_names(&issue_view(issue, "labels")?)
}

/// The REST API's path for the labels of `issue`.
fn labels_path(issue: &IssueUrl) -> String {
    format!("repos/{}/issues/{}/labels", issue.repo_slug(), issue.number)
}

/// Set `issue`'s labels to exactly `labels`, in one request, so a swap of
/// one label for another can't stop halfway. Through the REST API: `gh issue
/// edit` fails on the GitHub Projects (classic) sunset in older `gh`.
fn set_labels(issue: &IssueUrl, labels: &[&str]) -> Result<()> {
    let path = labels_path(issue);
    let fields: Vec<String> = labels
        .iter()
        .map(|label| format!("labels[]={label}"))
        .collect();
    let mut args = vec!["api", "--method", "PUT", &path, "--silent"];
    for field in &fields {
        args.extend(["-f", field]);
    }
    gh(&args)
}

/// Set `issue`'s labels to `kept` and then each of `added`, in one request,
/// as [`set_labels`] does. One of `kept` that is also one of `added`,
/// whatever its case, as GitHub's label names are case-insensitive, is there
/// once, as `added` spells it.
pub fn set_labels_adding(issue: &IssueUrl, kept: &[String], added: &[&str]) -> Result<()> {
    let mut labels: Vec<&str> = kept
        .iter()
        .map(String::as_str)
        .filter(|kept| !added.iter().any(|added| added.eq_ignore_ascii_case(kept)))
        .collect();
    labels.extend(added);
    set_labels(issue, &labels)
}

/// The REST API's path for `label` on `issue`.
fn label_path(issue: &IssueUrl, label: &str) -> String {
    format!("{}/{label}", labels_path(issue))
}

/// Take `label` off `issue`, in one request that keeps its other labels.
/// Through the REST API, as [`set_labels`] is. GitHub refuses it if the issue
/// does not have the label.
pub fn remove_label(issue: &IssueUrl, label: &str) -> Result<()> {
    gh(&[
        "api",
        "--method",
        "DELETE",
        &label_path(issue, label),
        "--silent",
    ])
}

/// The command that does what [`remove_label`] does, to run by hand.
pub fn remove_label_command(issue: &IssueUrl, label: &str) -> String {
    format!("gh api --method DELETE {}", label_path(issue, label))
}

/// The command that adds `label` to `issue`, keeping its other labels, to
/// run by hand: unlike [`set_labels`], it needs none of the others named.
pub fn add_label_command(issue: &IssueUrl, label: &str) -> String {
    format!(
        "gh api --method POST {} -f 'labels[]={label}'",
        labels_path(issue)
    )
}

/// A Spec's sub-issue, as a Spec run reads it.
pub struct Ticket {
    pub number: u64,
    pub is_open: bool,
    pub labels: Vec<String>,
    /// It has sub-issues of its own.
    pub has_sub_issues: bool,
    /// The numbers of the issues it is blocked by, in the Spec or not.
    pub blockers: Vec<u64>,
    /// The numbers of those blockers that are open.
    pub open_blockers: Vec<u64>,
}

/// Every sub-issue of `issue`, open or closed, with its labels, whether it
/// has sub-issues of its own and the issues it is blocked by: its GitHub
/// "blocked by" links, never the text of its body.
const TICKETS_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      subIssues(first: 100) {
        nodes {
          number
          state
          labels(first: 100) { nodes { name } }
          subIssues { totalCount }
          blockedBy(first: 100) { nodes { number state } }
        }
      }
    }
  }
}";

/// The sub-issues of `issue`, its Tickets if it has any, in one query.
pub fn tickets(issue: &IssueUrl) -> Result<Vec<Ticket>> {
    let json = issue_query(issue, TICKETS_QUERY)?;
    nodes(
        &json["data"]["repository"]["issue"]["subIssues"],
        "subIssues",
    )?
    .iter()
    .map(|node| {
        let mut blockers = Vec::new();
        let mut open_blockers = Vec::new();
        for blocker in nodes(&node["blockedBy"], "blockedBy")? {
            blockers.push(node_number(blocker)?);
            if node_is_open(blocker)? {
                open_blockers.push(node_number(blocker)?);
            }
        }
        Ok(Ticket {
            number: node_number(node)?,
            is_open: node_is_open(node)?,
            labels: nodes(&node["labels"], "labels")?
                .iter()
                .filter_map(|label| label["name"].as_str().map(String::from))
                .collect(),
            has_sub_issues: sub_issue_count(node)? > 0,
            blockers,
            open_blockers,
        })
    })
    .collect()
}

/// The answer to `query`, a GraphQL query about `issue` that takes its
/// `$owner`, `$repo` and `$number`.
fn issue_query(issue: &IssueUrl, query: &str) -> Result<Value> {
    gh_json(&[
        "api",
        "graphql",
        "-f",
        &format!("query={query}"),
        "-f",
        &format!("owner={}", issue.owner),
        "-f",
        &format!("repo={}", issue.repo),
        "-F",
        &format!("number={}", issue.number),
    ])
}

/// The `nodes` of `connection`, the `what` of a GraphQL answer.
fn nodes<'a>(connection: &'a Value, what: &str) -> Result<&'a Vec<Value>> {
    connection["nodes"]
        .as_array()
        .with_context(|| format!("gh api graphql output has no {what}"))
}

/// Whether the issue a GraphQL answer's `node` describes is open.
fn node_is_open(node: &Value) -> Result<bool> {
    Ok(node["state"]
        .as_str()
        .context("gh api graphql output has an issue with no state")?
        == "OPEN")
}

/// The number of the issue a GraphQL answer's `node` describes.
fn node_number(node: &Value) -> Result<u64> {
    node["number"]
        .as_u64()
        .context("gh api graphql output has an issue with no number")
}

/// How many sub-issues the issue a GraphQL answer's `node` describes has.
fn sub_issue_count(node: &Value) -> Result<u64> {
    node["subIssues"]["totalCount"]
        .as_u64()
        .context("gh api graphql output has an issue with no subIssues count")
}

/// Close `issue` with `comment`.
pub fn close_issue(issue: &IssueUrl, comment: &str) -> Result<()> {
    gh(&[
        "issue",
        "close",
        &issue.number.to_string(),
        "--repo",
        &issue.repo_slug(),
        "--comment",
        comment,
    ])
}

/// An issue as a listing of its repository's issues gives it.
pub struct ListedIssue {
    pub issue: IssueUrl,
    pub title: String,
    pub labels: Vec<String>,
}

/// Every open issue labelled `label` in the repository `repo`, an
/// `owner/repo`, newest first. They come from GitHub's issue list, not its
/// search, whose index can be a while behind an issue just opened.
pub fn open_issues_labelled(repo: &str, label: &str) -> Result<Vec<ListedIssue>> {
    issues_labelled(repo, label, "open")
}

/// Every closed issue labelled `label` in the repository `repo`, an
/// `owner/repo`, newest first.
pub fn closed_issues_labelled(repo: &str, label: &str) -> Result<Vec<ListedIssue>> {
    issues_labelled(repo, label, "closed")
}

/// Every issue labelled `label` in the repository `repo` whose state is
/// `state`, `open` or `closed`, newest first.
fn issues_labelled(repo: &str, label: &str, state: &str) -> Result<Vec<ListedIssue>> {
    let json = gh_json(&[
        "issue",
        "list",
        "--state",
        state,
        "--repo",
        repo,
        "--label",
        label,
        "--json",
        "url,title,labels",
        "--limit",
        "1000",
    ])?;
    json.as_array()
        .context("gh issue list did not return a list")?
        .iter()
        .map(|listed| {
            let field = |name: &str| {
                listed[name]
                    .as_str()
                    .with_context(|| format!("gh issue list output has no {name}"))
            };
            Ok(ListedIssue {
                issue: IssueUrl::parse(field("url")?)?,
                title: field("title")?.to_string(),
                labels: label_names(listed)?,
            })
        })
        .collect()
}

/// An open issue labelled for a Pickup run to take, as the Pickup run reads
/// what its listing doesn't give.
pub struct Candidate {
    /// The issue it is a sub-issue of, if it is one.
    pub parent: Option<IssueUrl>,
    /// It has sub-issues of its own.
    pub has_sub_issues: bool,
    /// The numbers of the open issues it is blocked by.
    pub open_blockers: Vec<u64>,
    /// The latest of the changes that shape it, if its timeline has one.
    pub last_shaped: Option<Shaped>,
}

/// A change that shapes an issue, and when it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shaped {
    pub by: Shaping,
    pub at: DateTime<Utc>,
}

/// What shapes an issue for a Pickup run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shaping {
    /// The label a Pickup run lists issues by was applied.
    Labelled,
    /// A sub-issue was added or removed.
    SubIssues,
    /// A "blocked by" link was added or removed.
    Blockers,
}

/// An issue's parent, whether it has sub-issues, the issues it is blocked
/// by, and the events of its timeline that shape it: each time a label was
/// applied, of which only the last hundred are read, and the last sub-issue
/// or "blocked by" link added or removed. They are read from the timeline
/// because adding a sub-issue does not change the issue's update time.
const CANDIDATE_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      parent { url }
      subIssues { totalCount }
      blockedBy(first: 100) { nodes { number state } }
      labelled: timelineItems(last: 100, itemTypes: [LABELED_EVENT]) {
        nodes { ... on LabeledEvent { createdAt label { name } } }
      }
      linked: timelineItems(
        last: 1
        itemTypes: [
          SUB_ISSUE_ADDED_EVENT
          SUB_ISSUE_REMOVED_EVENT
          BLOCKED_BY_ADDED_EVENT
          BLOCKED_BY_REMOVED_EVENT
        ]
      ) {
        nodes {
          __typename
          ... on SubIssueAddedEvent { createdAt }
          ... on SubIssueRemovedEvent { createdAt }
          ... on BlockedByAddedEvent { createdAt }
          ... on BlockedByRemovedEvent { createdAt }
        }
      }
    }
  }
}";

/// `issue` as a Pickup run that lists issues by `label` reads it, in one
/// query.
pub fn candidate(issue: &IssueUrl, label: &str) -> Result<Candidate> {
    let json = issue_query(issue, CANDIDATE_QUERY)?;
    let issue = &json["data"]["repository"]["issue"];
    let parent = issue["parent"]["url"]
        .as_str()
        .map(IssueUrl::parse)
        .transpose()?;
    let mut open_blockers = Vec::new();
    for blocker in nodes(&issue["blockedBy"], "blockedBy")? {
        if node_is_open(blocker)? {
            open_blockers.push(node_number(blocker)?);
        }
    }
    let at = |event: &Value| -> Result<DateTime<Utc>> {
        let at = event["createdAt"]
            .as_str()
            .context("gh api graphql output has an event with no createdAt")?;
        Ok(DateTime::parse_from_rfc3339(at)
            .with_context(|| format!("gh api graphql output has an unreadable createdAt {at}"))?
            .to_utc())
    };
    let mut shaped = Vec::new();
    for event in nodes(&issue["labelled"], "labelled")? {
        // GitHub's label names are case-insensitive.
        let name = event["label"]["name"].as_str();
        if name.is_some_and(|name| name.eq_ignore_ascii_case(label)) {
            shaped.push(Shaped {
                by: Shaping::Labelled,
                at: at(event)?,
            });
        }
    }
    for event in nodes(&issue["linked"], "linked")? {
        let by = match event["__typename"].as_str() {
            Some("SubIssueAddedEvent" | "SubIssueRemovedEvent") => Shaping::SubIssues,
            Some("BlockedByAddedEvent" | "BlockedByRemovedEvent") => Shaping::Blockers,
            other => bail!("gh api graphql output has an unknown event {other:?}"),
        };
        shaped.push(Shaped { by, at: at(event)? });
    }
    Ok(Candidate {
        parent,
        has_sub_issues: sub_issue_count(issue)? > 0,
        open_blockers,
        last_shaped: shaped.into_iter().max_by_key(|shaped| shaped.at),
    })
}

/// Add each of `labels` to the repository `repo`, an `owner/repo`, with its
/// description, unless the repository has it already.
pub fn ensure_labels(repo: &str, labels: &[(&str, &str)]) -> Result<()> {
    let known = gh_json(&[
        "label", "list", "--repo", repo, "--json", "name", "--limit", "1000",
    ])?;
    let known: Vec<&str> = known
        .as_array()
        .context("gh label list did not return a list")?
        .iter()
        .filter_map(|label| label["name"].as_str())
        .collect();
    for (name, description) in labels {
        // GitHub's label names are case-insensitive.
        if !known.iter().any(|known| known.eq_ignore_ascii_case(name)) {
            gh(&[
                "label",
                "create",
                name,
                "--repo",
                repo,
                "--description",
                description,
            ])?;
        }
    }
    Ok(())
}

/// Open an issue titled `title`, with `body` and `labels`, in the repository
/// of `issue`, and return it. Each label is first added to the repository,
/// with its description, if the repository lacks it: `gh` refuses a label it
/// doesn't know.
pub fn create_issue(
    issue: &IssueUrl,
    title: &str,
    body: &str,
    labels: &[(&str, &str)],
) -> Result<IssueUrl> {
    let repo = issue.repo_slug();
    ensure_labels(&repo, labels)?;
    let names: Vec<&str> = labels.iter().map(|(name, _)| *name).collect();
    let url = gh_stdout(&[
        "issue",
        "create",
        "--repo",
        &repo,
        "--title",
        title,
        "--body",
        body,
        "--label",
        &names.join(","),
    ])?;
    IssueUrl::parse(&url).context("gh issue create did not print the issue's URL")
}

/// The public email of the signed-in user's GitHub profile, or `None` if it
/// is private. Private addresses need the `user` scope, which a default
/// `gh auth login` token lacks, so thirdshift never asks for them.
pub fn profile_email() -> Result<Option<String>> {
    let json = gh_json(&["api", "user"])?;
    Ok(json["email"].as_str().map(str::to_string))
}

const PR_FIELDS: &str = "number,url,state,headRefName,baseRefName,isDraft";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl fmt::Display for PrState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
        })
    }
}

pub struct PullRequest {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub head: String,
    pub base: String,
    pub is_draft: bool,
}

impl PullRequest {
    pub fn is_open(&self) -> bool {
        self.state == PrState::Open
    }

    fn from_json(json: &Value) -> Result<Self> {
        let field = |name: &str| -> Result<String> {
            json[name]
                .as_str()
                .map(String::from)
                .with_context(|| format!("gh output has no {name}"))
        };
        let state = match field("state")?.as_str() {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            other => bail!("gh output has an unknown PR state {other}"),
        };
        Ok(PullRequest {
            number: json["number"].as_u64().context("gh output has no number")?,
            url: field("url")?,
            state,
            head: field("headRefName")?,
            base: field("baseRefName")?,
            is_draft: json["isDraft"]
                .as_bool()
                .context("gh output has no isDraft")?,
        })
    }
}

/// The `fields` of the pull request `pr`, its head branch or its number, as
/// JSON.
fn pr_view(issue: &IssueUrl, pr: &str, fields: &str) -> Result<Value> {
    gh_json(&[
        "pr",
        "view",
        pr,
        "--repo",
        &issue.repo_slug(),
        "--json",
        fields,
    ])
}

/// The pull request whose head is `branch`, if `gh` finds one.
pub fn pull_request_for(issue: &IssueUrl, branch: &str) -> Result<Option<PullRequest>> {
    let json = match pr_view(issue, branch, PR_FIELDS) {
        Ok(json) => json,
        Err(error) if format!("{error:#}").contains("no pull requests found") => return Ok(None),
        Err(error) => return Err(error),
    };
    PullRequest::from_json(&json).map(Some)
}

/// Every pull request in this repository, in any state, whose head branch
/// starts with `prefix`. PRs from forks are left out: their heads are not
/// this repository's branches.
pub fn pull_requests_with_head_prefix(issue: &IssueUrl, prefix: &str) -> Result<Vec<PullRequest>> {
    let json = gh_json(&[
        "pr",
        "list",
        "--repo",
        &issue.repo_slug(),
        "--state",
        "all",
        "--search",
        &format!("head:{prefix}"),
        "--limit",
        "1000",
        "--json",
        &format!("{PR_FIELDS},isCrossRepository"),
    ])?;
    json.as_array()
        .context("gh pr list did not return a list")?
        .iter()
        .filter(|pr| pr["isCrossRepository"] != Value::Bool(true))
        .map(PullRequest::from_json)
        .collect()
}

/// Open a draft pull request from `head` into `base`, and return its URL.
pub fn create_draft_pr(
    issue: &IssueUrl,
    head: &str,
    base: &str,
    title: &str,
    body: &str,
) -> Result<String> {
    gh_stdout(&[
        "pr",
        "create",
        "--repo",
        &issue.repo_slug(),
        "--head",
        head,
        "--base",
        base,
        "--title",
        title,
        "--body",
        body,
        "--draft",
    ])
}

/// The body of pull request `number`.
pub fn pr_body(issue: &IssueUrl, number: u64) -> Result<String> {
    let json = pr_view(issue, &number.to_string(), "body")?;
    Ok(json["body"]
        .as_str()
        .context("gh output has no body")?
        .to_string())
}

/// Set the body of pull request `number` to `body`, through the REST API:
/// `gh pr edit` fails on the GitHub Projects (classic) sunset in older `gh`.
pub fn set_pr_body(issue: &IssueUrl, number: u64, body: &str) -> Result<()> {
    gh(&[
        "api",
        "--method",
        "PATCH",
        &format!("repos/{}/pulls/{number}", issue.repo_slug()),
        "-f",
        &format!("body={body}"),
        "--silent",
    ])
}

/// Mark the pull request whose head is `branch` ready for review.
pub fn mark_ready(issue: &IssueUrl, branch: &str) -> Result<()> {
    gh(&["pr", "ready", branch, "--repo", &issue.repo_slug()])
}

/// Convert the pull request whose head is `branch` back to a draft.
pub fn convert_to_draft(issue: &IssueUrl, branch: &str) -> Result<()> {
    gh(&[
        "pr",
        "ready",
        branch,
        "--undo",
        "--repo",
        &issue.repo_slug(),
    ])
}

/// Merge the pull request whose head is `branch` into its base with a merge
/// commit, but only if its head is still `head`. Never GitHub's auto-merge
/// (ADR-0004), and never `--delete-branch`, whose local deletion would
/// interfere with the worktree. If `gh` fails but the pull request merged at
/// `head` anyway, e.g. because `gh` was interrupted once GitHub had merged,
/// that is a merge.
pub fn merge(issue: &IssueUrl, branch: &str, head: &str) -> Result<()> {
    let Err(error) = gh(&[
        "pr",
        "merge",
        branch,
        "--repo",
        &issue.repo_slug(),
        "--merge",
        "--match-head-commit",
        head,
    ]) else {
        return Ok(());
    };
    match merged_head(issue, branch) {
        Ok(Some(merged)) if merged == head => Ok(()),
        _ => Err(error),
    }
}

/// The head commit the pull request whose head is `branch` was merged at, if
/// it is merged.
fn merged_head(issue: &IssueUrl, branch: &str) -> Result<Option<String>> {
    let json = pr_view(issue, branch, "state,headRefOid")?;
    if json["state"] != "MERGED" {
        return Ok(None);
    }
    let head = json["headRefOid"]
        .as_str()
        .context("gh output has no headRefOid")?;
    Ok(Some(head.to_string()))
}

/// Whether a pull request can be merged into its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    /// It conflicts with its base.
    No,
    /// GitHub is still working it out.
    Unknown,
}

/// Whether the pull request whose head is `branch` can be merged.
pub fn mergeable(issue: &IssueUrl, branch: &str) -> Result<Mergeable> {
    let json = pr_view(issue, branch, "mergeable")?;
    match json["mergeable"].as_str() {
        Some("MERGEABLE") => Ok(Mergeable::Yes),
        Some("CONFLICTING") => Ok(Mergeable::No),
        Some("UNKNOWN") => Ok(Mergeable::Unknown),
        other => bail!("gh output has an unknown mergeable state {other:?}"),
    }
}

/// Where a check or status stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Pending,
    Passed,
    Failed,
}

/// A check run or a commit status.
pub struct Check {
    pub name: String,
    pub state: CheckState,
    pub url: Option<String>,
}

/// Every check run and commit status on commit `sha`.
pub fn checks_on(issue: &IssueUrl, sha: &str) -> Result<Vec<Check>> {
    let commit = format!("repos/{}/commits/{sha}", issue.repo_slug());
    let runs = gh_api_items(&format!("{commit}/check-runs?per_page=100"), "check_runs")?;
    let statuses = gh_api_items(&format!("{commit}/status?per_page=100"), "statuses")?;
    let url = |value: &Value| {
        value
            .as_str()
            .filter(|url| !url.is_empty())
            .map(String::from)
    };

    let mut checks = Vec::new();
    for run in &runs {
        let state = match (run["status"].as_str(), run["conclusion"].as_str()) {
            (Some("completed"), Some("success" | "neutral" | "skipped")) => CheckState::Passed,
            (Some("completed"), _) => CheckState::Failed,
            _ => CheckState::Pending,
        };
        checks.push(Check {
            name: run["name"]
                .as_str()
                .context("a check run has no name")?
                .to_string(),
            state,
            url: url(&run["details_url"]).or_else(|| url(&run["html_url"])),
        });
    }
    for status in &statuses {
        let state = match status["state"].as_str() {
            Some("success") => CheckState::Passed,
            Some("pending") => CheckState::Pending,
            _ => CheckState::Failed,
        };
        checks.push(Check {
            name: status["context"]
                .as_str()
                .context("a commit status has no context")?
                .to_string(),
            state,
            url: url(&status["target_url"]),
        });
    }
    Ok(checks)
}

/// Run `gh <args>`, failing with its stderr if it exits non-zero.
fn gh(args: &[&str]) -> Result<()> {
    gh_stdout(args).map(drop)
}

/// Run `gh <args>` and return its trimmed stdout, failing with its stderr if
/// it exits non-zero.
fn gh_stdout(args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Every item of the `field` list of the GitHub API's `path`, across all its
/// pages.
fn gh_api_items(path: &str, field: &str) -> Result<Vec<Value>> {
    let output = Command::new("gh")
        .args(["api", "--paginate", "--jq", &format!(".{field}[]"), path])
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "gh api {path} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::Deserializer::from_slice(&output.stdout)
        .into_iter()
        .collect::<Result<_, _>>()
        .with_context(|| format!("gh api {path} returned invalid JSON"))
}

/// Run `gh <args>` and parse its stdout as JSON, failing with gh's stderr if
/// it exits non-zero.
fn gh_json(args: &[&str]) -> Result<Value> {
    let command = format!("gh {}", args[..2].join(" "));
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh")?;
    if !output.status.success() {
        bail!(
            "{command} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("{command} returned invalid JSON"))
}
