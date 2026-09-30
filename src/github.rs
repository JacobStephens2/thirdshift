//! Asking GitHub, through `gh`, about issues, pull requests and the signed-in
//! user.

use std::fmt;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::issue::IssueUrl;

/// Whether `issue` is open.
pub fn issue_is_open(issue: &IssueUrl) -> Result<bool> {
    let json = gh_json(&[
        "issue",
        "view",
        &issue.number.to_string(),
        "--repo",
        &issue.repo_slug(),
        "--json",
        "state",
    ])?;
    Ok(json["state"].as_str().context("gh output has no state")? == "OPEN")
}

/// The title of `issue`.
pub fn issue_title(issue: &IssueUrl) -> Result<String> {
    let json = gh_json(&[
        "issue",
        "view",
        &issue.number.to_string(),
        "--repo",
        &issue.repo_slug(),
        "--json",
        "title",
    ])?;
    Ok(json["title"]
        .as_str()
        .context("gh output has no title")?
        .to_string())
}

/// A Spec's sub-issue, as a Spec run reads it.
pub struct Ticket {
    pub number: u64,
    pub is_open: bool,
    pub labels: Vec<String>,
    /// It has sub-issues of its own.
    pub has_sub_issues: bool,
    /// The numbers of the open issues it is blocked by, in the Spec or not.
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
    let json = gh_json(&[
        "api",
        "graphql",
        "-f",
        &format!("query={TICKETS_QUERY}"),
        "-f",
        &format!("owner={}", issue.owner),
        "-f",
        &format!("repo={}", issue.repo),
        "-F",
        &format!("number={}", issue.number),
    ])?;
    let nodes = |value: &Value, what: &str| -> Result<Vec<Value>> {
        value["nodes"]
            .as_array()
            .cloned()
            .with_context(|| format!("gh api graphql output has no {what}"))
    };
    let is_open = |node: &Value| -> Result<bool> {
        Ok(node["state"]
            .as_str()
            .context("gh api graphql output has an issue with no state")?
            == "OPEN")
    };
    let number = |node: &Value| {
        node["number"]
            .as_u64()
            .context("gh api graphql output has an issue with no number")
    };
    nodes(
        &json["data"]["repository"]["issue"]["subIssues"],
        "subIssues",
    )?
    .iter()
    .map(|node| {
        let mut open_blockers = Vec::new();
        for blocker in nodes(&node["blockedBy"], "blockedBy")? {
            if is_open(&blocker)? {
                open_blockers.push(number(&blocker)?);
            }
        }
        Ok(Ticket {
            number: number(node)?,
            is_open: is_open(node)?,
            labels: nodes(&node["labels"], "labels")?
                .iter()
                .filter_map(|label| label["name"].as_str().map(String::from))
                .collect(),
            has_sub_issues: node["subIssues"]["totalCount"]
                .as_u64()
                .context("gh api graphql output has an issue with no subIssues count")?
                > 0,
            open_blockers,
        })
    })
    .collect()
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

/// Whether merging the pull request whose head is `branch` closes `issue` by
/// itself. GitHub closes the issues a pull request links for closing, but
/// only when it merges into the repository's default branch, and a moment
/// after the merge rather than with it.
pub fn merge_closes_issue(issue: &IssueUrl, branch: &str) -> Result<bool> {
    let pr = pr_view(issue, branch, "baseRefName,closingIssuesReferences")?;
    let repo = gh_json(&[
        "repo",
        "view",
        &issue.repo_slug(),
        "--json",
        "defaultBranchRef",
    ])?;
    let links_issue = pr["closingIssuesReferences"]
        .as_array()
        .context("gh output has no closingIssuesReferences")?
        .iter()
        .any(|linked| linked["number"].as_u64() == Some(issue.number));
    Ok(links_issue && pr["baseRefName"] == repo["defaultBranchRef"]["name"])
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
