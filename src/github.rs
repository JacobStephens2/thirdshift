//! Asking GitHub, through `gh`, about issues, pull requests and the signed-in
//! user.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::issue::{IssueUrl, Repo};
use crate::labels::{Label, Labels};
use crate::process::{self, Control, Interruption};

mod check_read;
mod quota;
pub(crate) use check_read::is_transient as check_read_is_transient;
pub(crate) use quota::{Quota, Quotas};

mod advisories;
mod base_ci;
pub(crate) use base_ci::BaseCiAction;
mod session_issue;
mod trust;
pub use advisories::{
    DraftAdvisory, FindingDraft, FindingProvenance, Package, RecordedFinding, SecurityRecord,
    SecurityRecords,
};

#[cfg(test)]
mod execution_tests;
pub(crate) mod scoring;

/// A failed gh command keeps its diagnostic locally; notifications use only
/// its fixed operational category.
#[derive(Debug)]
enum CommandFailure {
    Rejected { command: String, stderr: String },
    InvalidJson { command: String },
    InvalidResponse(String),
    Execution,
}

impl CommandFailure {
    fn rejected(command: String, output: &Output) -> Self {
        Self::Rejected {
            command,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        }
    }
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected { command, stderr } => write!(f, "{command} failed: {stderr}"),
            Self::InvalidJson { command } => write!(f, "{command} returned invalid JSON"),
            Self::InvalidResponse(message) => f.write_str(message),
            Self::Execution => f.write_str("could not run gh"),
        }
    }
}

impl std::error::Error for CommandFailure {}

/// A fixed cause for failed GitHub queries, never their original diagnostics.
/// Only explicit quota messages establish exhaustion; a bare 403 does not.
pub(crate) fn operational_cause(error: &anyhow::Error) -> Option<&'static str> {
    let stderr = match error.downcast_ref::<CommandFailure>() {
        Some(CommandFailure::Rejected { stderr, .. }) => stderr,
        Some(_) => return Some("GitHub dependency failure"),
        None => &error.downcast_ref::<check_read::ReadFailure>()?.stderr,
    };
    let stderr = stderr.to_ascii_lowercase();
    Some(
        if stderr.contains("api rate limit exceeded")
            || stderr.contains("api rate limit already exceeded")
            || stderr.contains("secondary rate limit")
        {
            "GitHub rate-limit exhaustion"
        } else {
            "GitHub dependency failure"
        },
    )
}

/// GitHub CLI operations bound to an immutable interruption policy.
/// Ordinary views stop for Command interruption; completion views finish
/// independently without changing the recorded request or other views.
pub struct GitHub {
    interruption: Interruption,
}

impl GitHub {
    pub fn new() -> Self {
        Self {
            interruption: Interruption::Ordinary,
        }
    }

    pub fn completion(&self) -> Self {
        Self {
            interruption: Interruption::Completion,
        }
    }

    /// The `fields` of `issue`, as JSON.
    fn issue_view(&self, issue: &IssueUrl, fields: &str) -> Result<Value> {
        self.gh_json(&[
            "issue",
            "view",
            &issue.number.to_string(),
            "--repo",
            &issue.repo_slug(),
            "--json",
            fields,
        ])
    }

    /// Whether `issue` is open.
    pub fn issue_is_open(&self, issue: &IssueUrl) -> Result<bool> {
        state_is_open(&self.issue_view(issue, "state")?)
    }

    /// The title of `issue`.
    pub fn issue_title(&self, issue: &IssueUrl) -> Result<String> {
        let json = self.issue_view(issue, "title")?;
        Ok(json["title"]
            .as_str()
            .context(CommandFailure::InvalidResponse(
                "gh output has no title".into(),
            ))?
            .to_string())
    }

    /// `issue`'s title, state, labels and when it was created.
    pub fn issue(&self, issue: &IssueUrl) -> Result<Issue> {
        let json = self.issue_view(issue, "title,state,labels,createdAt")?;
        let created = json["createdAt"]
            .as_str()
            .context(CommandFailure::InvalidResponse(
                "gh output has no createdAt".into(),
            ))?;
        Ok(Issue {
            title: json["title"].as_str().map(String::from),
            is_open: state_is_open(&json)?,
            labels: labels_of(&json)?,
            created: DateTime::parse_from_rfc3339(created)
                .with_context(|| {
                    CommandFailure::InvalidResponse(format!(
                        "gh output has an unreadable createdAt {created}"
                    ))
                })?
                .to_utc(),
        })
    }

    /// The labels of `issue`.
    pub fn issue_labels(&self, issue: &IssueUrl) -> Result<Labels> {
        labels_of(&self.issue_view(issue, "labels")?)
    }

    /// Set `issue`'s labels to exactly `labels`, in one request, so a swap of
    /// one label for another can't stop halfway. Through the REST API: `gh issue
    /// edit` fails on the GitHub Projects (classic) sunset in older `gh`.
    pub fn set_labels(&self, issue: &IssueUrl, labels: &Labels) -> Result<()> {
        let path = labels_path(issue);
        let fields: Vec<String> = labels
            .names()
            .map(|label| format!("labels[]={label}"))
            .collect();
        let mut args = vec!["api", "--method", "PUT", &path, "--silent"];
        for field in &fields {
            args.extend(["-f", field]);
        }
        self.gh(&args)
    }

    /// Take `label` off `issue`, in one request that keeps its other labels.
    /// Through the REST API, as [`GitHub::set_labels`] is. GitHub refuses it if the issue
    /// does not have the label.
    pub fn remove_label(&self, issue: &IssueUrl, label: &str) -> Result<()> {
        self.gh(&[
            "api",
            "--method",
            "DELETE",
            &label_path(issue, label),
            "--silent",
        ])
    }

    /// The sub-issues of `issue`, its Tickets if it has any, in one query.
    pub fn tickets(&self, issue: &IssueUrl) -> Result<Vec<Ticket>> {
        let json = self.issue_query(issue, TICKETS_QUERY, None)?;
        nodes(
            &json["data"]["repository"]["issue"]["subIssues"],
            "subIssues",
        )?
        .iter()
        .map(|node| {
            let mut blockers = Vec::new();
            let mut open_blockers = Vec::new();
            for blocker in nodes(&node["blockedBy"], "blockedBy")? {
                blockers.push(node_issue(blocker)?);
                if node_is_open(blocker)? {
                    let issue = node_issue(blocker)?;
                    let labels = &blocker["labels"];
                    let more_labels = labels["pageInfo"]["hasNextPage"].as_bool().context(
                        CommandFailure::InvalidResponse(
                            "blocker labels have no hasNextPage".into(),
                        ),
                    )?;
                    open_blockers.push(TicketBlocker {
                        labels: if more_labels {
                            self.issue_labels(&issue)?
                        } else {
                            named(nodes(labels, "labels")?)
                        },
                        issue,
                        title: blocker["title"].as_str().map(String::from),
                    });
                }
            }
            Ok(Ticket {
                issue: node_issue(node)?,
                title: node["title"].as_str().map(String::from),
                is_open: node_is_open(node)?,
                labels: named(nodes(&node["labels"], "labels")?),
                has_sub_issues: sub_issue_count(node)? > 0,
                blockers,
                open_blockers,
            })
        })
        .collect()
    }

    /// The answer to `query`, a GraphQL query about `issue` that takes its
    /// `$owner`, `$repo` and `$number`, and an optional label-history cursor.
    fn issue_query(&self, issue: &IssueUrl, query: &str, after: Option<&str>) -> Result<Value> {
        let query = format!("query={query}");
        let owner = format!("owner={}", issue.owner);
        let repo = format!("repo={}", issue.repo);
        let number = format!("number={}", issue.number);
        let mut args = vec![
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &repo, "-F", &number,
        ];
        let cursor;
        if let Some(after) = after {
            cursor = format!("after={after}");
            args.extend(["-f", &cursor]);
        }
        self.gh_json(&args)
    }

    /// Every node of `issue`'s `connection`, the `what` that `query` reads
    /// page by page through its `$after` cursor, with the last page's answer.
    fn paged_nodes(
        &self,
        issue: &IssueUrl,
        query: &str,
        connection: &str,
        what: &str,
    ) -> Result<(Value, Vec<Value>)> {
        let mut after = None;
        let mut all = Vec::new();
        loop {
            let json = self.issue_query(issue, query, after.as_deref())?;
            let page = &json["data"]["repository"]["issue"][connection];
            all.extend(nodes(page, connection)?.iter().cloned());
            let info = &page["pageInfo"];
            if !info["hasNextPage"].as_bool().with_context(|| {
                CommandFailure::InvalidResponse(format!("{what} has no hasNextPage"))
            })? {
                return Ok((json, all));
            }
            let next = info["endCursor"].as_str().with_context(|| {
                CommandFailure::InvalidResponse(format!("{what} has no endCursor"))
            })?;
            if Some(next) == after.as_deref() {
                bail!(CommandFailure::InvalidResponse(format!(
                    "{what} cursor did not advance"
                )));
            }
            after = Some(next.to_string());
        }
    }

    /// Close `issue` with `comment`.
    pub fn close_issue(&self, issue: &IssueUrl, comment: &str) -> Result<()> {
        self.gh(&[
            "issue",
            "close",
            &issue.number.to_string(),
            "--repo",
            &issue.repo_slug(),
            "--comment",
            comment,
        ])
    }

    /// Comment on `issue`, which is already closed.
    pub fn comment_issue(&self, issue: &IssueUrl, comment: &str) -> Result<()> {
        self.gh(&[
            "issue",
            "comment",
            &issue.number.to_string(),
            "--repo",
            &issue.repo_slug(),
            "--body",
            comment,
        ])
    }

    /// The numeric database id of `issue`, for API calls that take one
    /// instead of the issue number (sub-issues, dependencies).
    fn issue_db_id(&self, issue: &IssueUrl) -> Result<u64> {
        let json = self.gh_json(&[
            "api",
            &format!("repos/{}/issues/{}", issue.repo_slug(), issue.number),
            "--jq",
            ".id",
        ])?;
        json.as_u64().context(CommandFailure::InvalidResponse(
            "gh api did not return the issue's database id".into(),
        ))
    }

    /// Link `child` as a native sub-issue of `parent`.
    pub fn add_sub_issue(&self, parent: &IssueUrl, child: &IssueUrl) -> Result<()> {
        let child_id = self.issue_db_id(child)?;
        self.post_by_id(
            parent,
            &format!("issues/{}/sub_issues", parent.number),
            "sub_issue_id",
            child_id,
        )
    }

    /// Add a native "blocked by" edge: `ticket` is blocked by `blocker`.
    pub fn add_blocked_by(&self, ticket: &IssueUrl, blocker: &IssueUrl) -> Result<()> {
        let blocker_id = self.issue_db_id(blocker)?;
        self.post_by_id(
            ticket,
            &format!("issues/{}/dependencies/blocked_by", ticket.number),
            "issue_id",
            blocker_id,
        )
    }

    /// `POST` one database-id field to the `endpoint` below
    /// `repos/<owner>/<repo>`: the shared shape behind issue links that take
    /// an id instead of an issue number.
    fn post_by_id(&self, issue: &IssueUrl, endpoint: &str, field: &str, id: u64) -> Result<()> {
        self.gh(&[
            "api",
            "--method",
            "POST",
            &format!("repos/{}/{}", issue.repo_slug(), endpoint),
            "-F",
            &format!("{field}={id}"),
        ])
    }

    /// Every open issue labelled `label` in the repository `repo`, an
    /// `owner/repo`, newest first. Read every page of the repository REST
    /// issues endpoint: `gh issue list --label` uses search, whose index can
    /// be a while behind an issue just opened. Pull requests are excluded.
    pub fn open_issues_labelled(&self, repo: &str, label: Label) -> Result<Vec<ListedIssue>> {
        self.issues_labelled(repo, label, "open")
    }

    /// Every closed issue labelled `label` in the repository `repo`, an
    /// `owner/repo`, newest first.
    pub fn closed_issues_labelled(&self, repo: &str, label: Label) -> Result<Vec<ListedIssue>> {
        self.issues_labelled(repo, label, "closed")
    }

    /// Every issue labelled `label` in the repository `repo` whose state is
    /// `state`, `open` or `closed`, newest first.
    fn issues_labelled(&self, repo: &str, label: Label, state: &str) -> Result<Vec<ListedIssue>> {
        let labels = serde_urlencoded::to_string([("labels", label.name())])?;
        let issues = self.gh_api_items(
            &format!(
                "repos/{repo}/issues?state={state}&{labels}&sort=created&direction=desc&per_page=100"
            ),
            "",
        )?;
        issues
            .iter()
            .filter(|listed| listed.get("pull_request").is_none())
            .map(|listed| {
                let field = |name: &str| {
                    listed[name].as_str().with_context(|| {
                        CommandFailure::InvalidResponse(format!("gh api output has no {name}"))
                    })
                };
                Ok(ListedIssue {
                    issue: IssueUrl::parse(field("html_url")?).context(
                        CommandFailure::InvalidResponse(
                            "gh api output has an invalid issue URL".into(),
                        ),
                    )?,
                    title: field("title")?.to_string(),
                    labels: labels_of(listed)?,
                })
            })
            .collect()
    }

    /// Every issue in the repository `repo`, an `owner/repo`, created at or
    /// after `since`, open or closed, regardless of author. Security fix
    /// publication checks their text before deciding which are leaks.
    /// Compared at whole seconds, as the `created` check on a fix Ticket is.
    pub fn issues_created_since(&self, repo: &str, since: DateTime<Utc>) -> Result<Vec<IssueUrl>> {
        let mut found = Vec::new();
        for state in ["open", "closed"] {
            let json = self.gh_json(&[
                "issue",
                "list",
                "--state",
                state,
                "--repo",
                repo,
                "--json",
                "url,createdAt",
                "--limit",
                "1000",
            ])?;
            for listed in json.as_array().context(CommandFailure::InvalidResponse(
                "gh issue list did not return a list".into(),
            ))? {
                let url = listed["url"]
                    .as_str()
                    .context(CommandFailure::InvalidResponse(
                        "gh issue list output has no url".into(),
                    ))?;
                let at = listed["createdAt"]
                    .as_str()
                    .context(CommandFailure::InvalidResponse(
                        "gh issue list output has no createdAt".into(),
                    ))?;
                let at = DateTime::parse_from_rfc3339(at)
                    .with_context(|| {
                        CommandFailure::InvalidResponse(format!(
                            "gh issue list output has an unreadable createdAt {at}"
                        ))
                    })?
                    .to_utc();
                if at.timestamp() >= since.timestamp() {
                    found.push(
                        IssueUrl::parse(url).context(CommandFailure::InvalidResponse(
                            "gh issue list output has an invalid issue URL".into(),
                        ))?,
                    );
                }
            }
        }
        Ok(found)
    }

    /// `issue` as a Pickup run that lists issues by `label` reads it, in one
    /// query per page of label history, oldest first. Every page is read so
    /// missing older Claims cannot be mistaken for never-Claimed work.
    pub fn candidate(&self, issue: &IssueUrl, label: Label) -> Result<Candidate> {
        let (json, label_events) =
            self.paged_nodes(issue, CANDIDATE_QUERY, "labelled", "label history")?;
        let issue = &json["data"]["repository"]["issue"];
        let parent = issue["parent"]["url"]
            .as_str()
            .map(IssueUrl::parse)
            .transpose()
            .context(CommandFailure::InvalidResponse(
                "gh api graphql output has an invalid parent issue URL".into(),
            ))?;
        let mut open_blockers = Vec::new();
        for blocker in nodes(&issue["blockedBy"], "blockedBy")? {
            if node_is_open(blocker)? {
                open_blockers.push(node_number(blocker)?);
            }
        }
        let at = |event: &Value| -> Result<DateTime<Utc>> {
            let at = event["createdAt"]
                .as_str()
                .context(CommandFailure::InvalidResponse(
                    "gh api graphql output has an event with no createdAt".into(),
                ))?;
            Ok(DateTime::parse_from_rfc3339(at)
                .with_context(|| {
                    CommandFailure::InvalidResponse(format!(
                        "gh api graphql output has an unreadable createdAt {at}"
                    ))
                })?
                .to_utc())
        };
        let mut shaped = Vec::new();
        let mut history = ClaimHistory::NeverClaimed;
        let mut claim_generation = 0;
        let mut label_events = label_events
            .iter()
            .map(|event| {
                let applied = match event["__typename"].as_str() {
                    Some("LabeledEvent") => true,
                    Some("UnlabeledEvent") => false,
                    other => bail!(CommandFailure::InvalidResponse(format!(
                        "gh api graphql output has an unknown label event {other:?}"
                    ))),
                };
                Ok((at(event)?, event["label"]["name"].as_str(), applied))
            })
            .collect::<Result<Vec<_>>>()?;
        // GitHub can list one label swap in either order within a second.
        // Replay Claims before ready labels at each timestamp, preserving the
        // order between Claim applications and removals so a run can still
        // release its own Claim. An active Claim cannot become a hand-back.
        label_events
            .sort_by_key(|(at, name, _)| (*at, name.is_some_and(|name| label.is_named(name))));
        for (at, name, applied) in label_events {
            if applied && name.is_some_and(|name| crate::claim::IN_PROGRESS.is_named(name)) {
                claim_generation += 1;
            }
            history = history.after(name, applied, label);
            if applied && name.is_some_and(|name| label.is_named(name)) {
                shaped.push(Shaped {
                    by: Shaping::Labelled,
                    at,
                });
            }
        }
        for event in nodes(&issue["linked"], "linked")? {
            let by = match event["__typename"].as_str() {
                Some("SubIssueAddedEvent" | "SubIssueRemovedEvent") => Shaping::SubIssues,
                Some("BlockedByAddedEvent" | "BlockedByRemovedEvent") => Shaping::Blockers,
                other => bail!(CommandFailure::InvalidResponse(format!(
                    "gh api graphql output has an unknown event {other:?}"
                ))),
            };
            shaped.push(Shaped { by, at: at(event)? });
        }
        Ok(Candidate {
            parent,
            sub_issue_is_open: nodes(&issue["subIssues"], "subIssues")?
                .iter()
                .map(node_is_open)
                .collect::<Result<_>>()?,
            open_blockers,
            last_shaped: shaped.into_iter().max_by_key(|shaped| shaped.at),
            handed_back: matches!(history, ClaimHistory::HandedBack),
            claim_generation,
            claimed: matches!(history, ClaimHistory::Claimed),
        })
    }

    /// Add each of `labels` to the repository `repo`, an `owner/repo`, with its
    /// description, unless the repository has it already.
    pub fn ensure_labels(&self, repo: &str, labels: &[Label]) -> Result<()> {
        let known = self.gh_json(&[
            "label", "list", "--repo", repo, "--json", "name", "--limit", "1000",
        ])?;
        let known = named(known.as_array().context(CommandFailure::InvalidResponse(
            "gh label list did not return a list".into(),
        ))?);
        for label in labels {
            if !known.has(*label) {
                self.gh(&[
                    "label",
                    "create",
                    label.name(),
                    "--repo",
                    repo,
                    "--description",
                    label.description(),
                ])?;
            }
        }
        Ok(())
    }

    /// Open an issue titled `title`, with `body` and `labels`, in the repository
    /// of `issue`, and return it. See [`GitHub::create_issue_in`].
    pub fn create_issue(
        &self,
        issue: &IssueUrl,
        title: &str,
        body: &str,
        labels: &[Label],
    ) -> Result<IssueUrl> {
        self.create_issue_in(&issue.repo(), title, body, labels)
    }

    /// Open an issue titled `title`, with `body` and `labels`, in `repo`,
    /// and return it. Each label is first added to the repository,
    /// with its description, if the repository lacks it: `gh` refuses a label it
    /// doesn't know.
    pub fn create_issue_in(
        &self,
        repo: &Repo,
        title: &str,
        body: &str,
        labels: &[Label],
    ) -> Result<IssueUrl> {
        let repo = repo.slug();
        self.ensure_labels(&repo, labels)?;
        let names: Vec<&str> = labels.iter().map(|label| label.name()).collect();
        let url = self.gh_stdout(&[
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
        IssueUrl::parse(&url).context(CommandFailure::InvalidResponse(
            "gh issue create did not print the issue's URL".into(),
        ))
    }

    /// The public email of the signed-in user's GitHub profile, or `None` if it
    /// is private. Private addresses need the `user` scope, which a default
    /// `gh auth login` token lacks, so thirdshift never asks for them.
    pub fn profile_email(&self) -> Result<Option<String>> {
        let json = self.gh_json(&["api", "user"])?;
        Ok(json["email"].as_str().map(str::to_string))
    }

    /// The `fields` of the pull request `pr`, its head branch or its number, as
    /// JSON.
    fn pr_view(&self, issue: &IssueUrl, pr: &str, fields: &str) -> Result<Value> {
        self.gh_json(&[
            "pr",
            "view",
            pr,
            "--repo",
            &issue.repo_slug(),
            "--json",
            fields,
        ])
    }

    /// A PR JSON read where GitHub's missing-PR diagnostic means absence.
    /// Active cancellation and all other transport failures still propagate.
    fn optional_pr_view(
        &self,
        issue: &IssueUrl,
        selector: &str,
        fields: &str,
    ) -> Result<Option<Value>> {
        match self.pr_view(issue, selector, fields) {
            Ok(json) => Ok(Some(json)),
            Err(error) if format!("{error:#}").contains("no pull requests found") => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A coherent pull request observation, selected by head branch for initial
    /// discovery and by number after capture. Policy belongs to the pull
    /// request owner.
    pub fn pr_snapshot(&self, issue: &IssueUrl, selector: &str) -> Result<Option<PrSnapshot>> {
        let Some(json) = self.optional_pr_view(issue, selector, SNAPSHOT_FIELDS)? else {
            return Ok(None);
        };
        let mergeable = match json["mergeable"].as_str() {
            Some("MERGEABLE") => Mergeable::Yes,
            Some("CONFLICTING") => Mergeable::No,
            Some("UNKNOWN") => Mergeable::Unknown,
            other => bail!(CommandFailure::InvalidResponse(format!(
                "gh output has an unknown mergeable state {other:?}"
            ))),
        };
        Ok(Some(PrSnapshot {
            pr: PullRequest::from_json(&json)?,
            mergeable,
            head_commit: json["headRefOid"]
                .as_str()
                .context(CommandFailure::InvalidResponse(
                    "gh output has no headRefOid".into(),
                ))?
                .to_string(),
            from_fork: json["isCrossRepository"].as_bool().context(
                CommandFailure::InvalidResponse("gh output has no isCrossRepository".into()),
            )?,
        }))
    }

    /// Every pull request in this repository, in any state, whose head branch
    /// starts with `prefix`. PRs from forks are left out: their heads are not
    /// this repository's branches.
    pub fn pull_requests_with_head_prefix(
        &self,
        issue: &IssueUrl,
        prefix: &str,
    ) -> Result<Vec<PullRequest>> {
        let json = self.gh_json(&[
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
            .context(CommandFailure::InvalidResponse(
                "gh pr list did not return a list".into(),
            ))?
            .iter()
            .filter(|pr| pr["isCrossRepository"] != Value::Bool(true))
            .map(PullRequest::from_json)
            .collect()
    }

    /// What GitHub last showed for one batch of up to a hundred issues and pull requests.
    /// The Inbox checks the returned quota before asking for another batch. Each
    /// alias asks for state and settlement times (plus issue triage labels),
    /// and for the query's own `rateLimit`. A subject GitHub can't
    /// resolve answers null and stays out of the map, without failing the
    /// rest of the read. Only reads; nothing here writes to GitHub.
    pub(crate) fn inbox_states(
        &self,
        urls: &[String],
    ) -> Result<(BTreeMap<String, InboxState>, Quota)> {
        let mut states = BTreeMap::new();
        let mut query = String::from("query {");
        let mut aliases = Vec::new();
        for (index, url) in urls.iter().enumerate() {
            let subject = split_pull_request_url(url)
                .map(|(owner, repo, number)| (owner, repo, number, InboxSubject::PullRequest))
                .or_else(|| {
                    IssueUrl::parse(url)
                        .ok()
                        .map(|issue| (issue.owner, issue.repo, issue.number, InboxSubject::Issue))
                });
            let Some((owner, repo, number, subject)) = subject else {
                continue;
            };
            // A record from another account reaches this query unchanged.
            // Reject anything that is not a GitHub name before interpolating it.
            if !github_name(&owner) || !github_name(&repo) {
                continue;
            }
            let alias = format!("pr{index}");
            let (field, fields) = subject.selection();
            query.push_str(&format!(" {alias}: repository(owner: \"{owner}\", name: \"{repo}\") {{ {field}(number: {number}) {{ {fields} }} }}"));
            aliases.push((alias, url.clone(), subject));
        }
        query.push_str(" rateLimit { limit cost remaining resetAt } }");
        let query = format!("query={query}");
        // gh exits unsuccessfully for scoped GraphQL errors even when
        // stdout contains healthy siblings. Only those partial answers
        // are usable; a transport or query-wide failure remains a failed
        // read so the Inbox keeps its previous answer.
        let json = self.retry_read("gh api graphql", || {
            let output = self.output(&["api", "graphql", "-f", &query])?;
            let json = serde_json::from_slice::<Value>(&output.stdout);
            let partial = json.as_ref().is_ok_and(|json| {
                json["data"].is_object()
                    && json["errors"].as_array().is_some_and(|errors| {
                        !errors.is_empty()
                            && errors.iter().all(|error| {
                                error["path"][0].as_str().is_some_and(|path| {
                                    aliases.iter().any(|(alias, _, _)| alias == path)
                                })
                            })
                    })
            });
            if !output.status.success() && !partial {
                bail!(check_read::ReadFailure {
                    command: "gh api graphql".into(),
                    stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    private_status: None,
                });
            }
            json.context(CommandFailure::InvalidJson {
                command: "gh api graphql".into(),
            })
        })?;
        let data = &json["data"];
        let quota = Quota::graphql(&data["rateLimit"])?;
        for (alias, url, subject) in aliases {
            if json["errors"]
                .as_array()
                .is_some_and(|errors| errors.iter().any(|error| error["path"][0] == alias))
            {
                continue;
            }
            let (field, _) = subject.selection();
            let pull = &data[alias.as_str()][field];
            if pull.is_null() {
                continue;
            }
            states.insert(url.clone(), subject.read(&url, pull)?);
        }
        Ok((states, quota))
    }

    /// Open a draft pull request from `head` into `base`, and return its URL.
    pub fn create_draft_pr(
        &self,
        issue: &IssueUrl,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<String> {
        self.gh_stdout(&[
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
    pub fn pr_body(&self, issue: &IssueUrl, number: u64) -> Result<String> {
        let json = self.pr_view(issue, &number.to_string(), "body")?;
        Ok(json["body"]
            .as_str()
            .context(CommandFailure::InvalidResponse(
                "gh output has no body".into(),
            ))?
            .to_string())
    }

    /// Set the body of pull request `number` to `body`, through the REST API:
    /// `gh pr edit` fails on the GitHub Projects (classic) sunset in older `gh`.
    pub fn set_pr_body(&self, issue: &IssueUrl, number: u64, body: &str) -> Result<()> {
        self.gh(&[
            "api",
            "--method",
            "PATCH",
            &format!("repos/{}/pulls/{number}", issue.repo_slug()),
            "-f",
            &format!("body={body}"),
            "--silent",
        ])
    }

    /// Mark the selected pull request ready for review.
    pub fn mark_ready(&self, issue: &IssueUrl, selector: &str) -> Result<()> {
        self.gh(&["pr", "ready", selector, "--repo", &issue.repo_slug()])
    }

    /// Convert the selected pull request back to a draft. Spec PR creation
    /// also uses this before Delivery has captured its identity.
    pub fn convert_to_draft(&self, issue: &IssueUrl, selector: &str) -> Result<()> {
        self.gh(&[
            "pr",
            "ready",
            selector,
            "--undo",
            "--repo",
            &issue.repo_slug(),
        ])
    }

    /// Whether a structured review decision blocks this numbered PR's merge.
    /// Failed or unrecognized reads are unknown and leave the merge to GitHub.
    pub fn review_blocks_merge(&self, issue: &IssueUrl, number: u64) -> bool {
        self.pr_view(issue, &number.to_string(), "reviewDecision")
            .ok()
            .is_some_and(|json| {
                matches!(
                    json["reviewDecision"].as_str(),
                    Some("REVIEW_REQUIRED" | "CHANGES_REQUESTED")
                )
            })
    }

    /// One guarded merge request, with a merge commit and without auto-merge
    /// or local branch deletion (ADR 0004). Delivery owns reconciliation.
    pub fn merge(&self, issue: &IssueUrl, number: u64, head: &str) -> Result<()> {
        self.gh(&[
            "pr",
            "merge",
            &number.to_string(),
            "--repo",
            &issue.repo_slug(),
            "--merge",
            "--match-head-commit",
            head,
        ])
    }

    /// Current check runs and commit statuses on `sha`, with conclusions as
    /// GitHub named them. Only the newest run of each Actions workflow counts;
    /// GitHub itself selects the latest check runs within a run's attempts.
    pub fn checks_on(&self, issue: &IssueUrl, sha: &str) -> Result<Vec<Check>> {
        let commit = format!("repos/{}/commits/{sha}", issue.repo_slug());
        // CI retries the complete observation, not its individual requests.
        let runs = self.check_runs_on(issue, sha)?;
        let statuses =
            self.gh_api_items_once(&format!("{commit}/status?per_page=100"), "statuses")?;
        let url = |value: &Value| {
            value
                .as_str()
                .filter(|url| !url.is_empty())
                .map(String::from)
        };

        let mut checks = Vec::new();
        for run in &runs.records {
            let conclusion = match run["status"].as_str() {
                Some("completed") => Conclusion::of_check_run(run["conclusion"].as_str()),
                _ => Conclusion::Unfinished,
            };
            checks.push(Check {
                name: run["name"]
                    .as_str()
                    .context(CommandFailure::InvalidResponse(
                        "a check run has no name".into(),
                    ))?
                    .to_string(),
                conclusion,
                job: ActionsJob::of(run["id"].as_u64(), run["details_url"].as_str()),
                workflow_run: None,
                url: url(&run["details_url"]).or_else(|| url(&run["html_url"])),
            });
        }
        for status in &statuses {
            let conclusion = Conclusion::of_commit_status(status["state"].as_str());
            checks.push(Check {
                name: status["context"]
                    .as_str()
                    .context(CommandFailure::InvalidResponse(
                        "a commit status has no context".into(),
                    ))?
                    .to_string(),
                conclusion,
                url: url(&status["target_url"]),
                job: None,
                workflow_run: None,
            });
        }
        let workflows = match runs.workflows {
            Some(workflows) => Some(workflows),
            None if checks.iter().any(|check| check.job.is_some()) => {
                Some(self.workflow_runs_on(issue, sha)?)
            }
            None => None,
        };
        if let Some(workflows) = workflows {
            Self::exclude_superseded_runs(&workflows, &mut checks);
        }
        Ok(checks)
    }

    /// Check-run shaped records, using Actions jobs when Checks is forbidden.
    /// Both CI watch and Score packets use this read and supersession rule.
    fn check_runs_on(&self, issue: &IssueUrl, sha: &str) -> Result<CheckRuns> {
        let path = format!(
            "repos/{}/commits/{sha}/check-runs?per_page=100",
            issue.repo_slug()
        );
        match self.gh_api_items_once(&path, "check_runs") {
            Ok(records) => {
                return Ok(CheckRuns {
                    records,
                    workflows: None,
                });
            }
            Err(error)
                if error
                    .downcast_ref::<check_read::ReadFailure>()
                    .is_some_and(|failure| failure.http_status() == Some(403)) => {}
            Err(error) => return Err(error),
        }
        let workflows = self.workflow_runs_on(issue, sha)?;
        let mut checks = Vec::new();
        for run in workflows.iter().filter(|run| !run.superseded) {
            let path = format!(
                "repos/{}/actions/runs/{}/jobs?per_page=100",
                issue.repo_slug(),
                run.id
            );
            for mut job in self.gh_api_items_once(&path, "jobs")? {
                job["details_url"] = job["html_url"].clone();
                checks.push(job);
            }
        }
        Ok(CheckRuns {
            records: checks,
            workflows: Some(workflows),
        })
    }

    /// Ignore checks of earlier workflow runs on this same commit, including
    /// jobs whose names the newest run no longer uses. A re-run keeps its run
    /// id and creation time, so its attempts are never superseded here.
    fn exclude_superseded_runs(runs: &[ListedRun], checks: &mut Vec<Check>) {
        let superseded: HashSet<_> = runs
            .iter()
            .filter(|run| run.superseded)
            .map(|run| run.id)
            .collect();
        checks.retain(|check| {
            check
                .job
                .is_none_or(|job| !superseded.contains(&job.workflow_run))
        });
        for check in checks.iter_mut() {
            if let Some(job) = check.job {
                check.workflow_run =
                    runs.iter()
                        .find(|run| run.id == job.workflow_run)
                        .map(|run| WorkflowRun {
                            id: run.id,
                            workflow_id: run.workflow_id,
                        });
            }
        }
        Self::include_unobserved_workflows(runs, checks);
    }

    fn workflow_runs_on(&self, issue: &IssueUrl, sha: &str) -> Result<Vec<ListedRun>> {
        let path = format!(
            "repos/{}/actions/runs?head_sha={sha}&per_page=100",
            issue.repo_slug()
        );
        let runs = self.gh_api_items_once(&path, "workflow_runs")?;
        let mut runs: Vec<ListedRun> = runs
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .context(CommandFailure::InvalidResponse(
                "unreadable workflow run".into(),
            ))?;
        let mut latest = HashMap::new();
        for run in &runs {
            let created = DateTime::parse_from_rfc3339(&run.created_at).context(
                CommandFailure::InvalidResponse("unreadable workflow creation time".into()),
            )?;
            let candidate = (created, run.id);
            let selected = latest.entry(run.workflow_id).or_insert(candidate);
            *selected = (*selected).max(candidate);
        }
        for run in &mut runs {
            run.superseded = latest[&run.workflow_id].1 != run.id;
        }
        Ok(runs)
    }

    fn include_unobserved_workflows(runs: &[ListedRun], checks: &mut Vec<Check>) {
        // The two reads are separate observations. A new run may have none of
        // its jobs yet, or only jobs that already finished while a later one
        // has not been created. Until the run has completed, that gap must
        // stay pending: the older run's checks are already gone, and a
        // listing of only finished successes is CI passed.
        for run in runs {
            if run.superseded {
                continue;
            }
            let mut saw_job = false;
            let mut every_job_passed = true;
            for check in checks.iter() {
                if check.job.is_some_and(|job| job.workflow_run == run.id) {
                    saw_job = true;
                    every_job_passed &= matches!(
                        check.conclusion,
                        Conclusion::Success | Conclusion::Neutral | Conclusion::Skipped
                    );
                }
            }
            // A present status other than completed is still running. An
            // absent one leaves the jobs to speak: GitHub sends a status.
            let open = matches!(run.status.as_deref(), Some(status) if status != "completed");
            if saw_job && (!open || !every_job_passed) {
                continue;
            }
            checks.push(Check {
                name: run
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("workflow {}", run.workflow_id)),
                conclusion: if run.status.as_deref() == Some("completed") {
                    Conclusion::of_check_run(run.conclusion.as_deref())
                } else {
                    Conclusion::Unfinished
                },
                url: run.html_url.clone(),
                job: None,
                workflow_run: Some(WorkflowRun {
                    id: run.id,
                    workflow_id: run.workflow_id,
                }),
            });
        }
    }

    /// The GitHub Actions job whose check run is `check_run`, as the jobs
    /// API reports it: its conclusion and each of its steps'.
    pub fn job_run(&self, issue: &IssueUrl, check_run: u64) -> Result<JobRun> {
        let path = format!("repos/{}/actions/jobs/{check_run}", issue.repo_slug());
        let job = self.gh_json(&["api", &path])?;
        let conclusion = |value: &Value| match value["status"].as_str() {
            Some("completed") => Conclusion::of_check_run(value["conclusion"].as_str()),
            _ => Conclusion::Unfinished,
        };
        let steps = job["steps"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        Ok(JobRun {
            conclusion: conclusion(&job),
            steps: steps.iter().map(conclusion).collect(),
        })
    }

    /// Ask GitHub to re-run the failed jobs of the GitHub Actions workflow run
    /// `workflow_run`, as a new attempt of it.
    pub fn rerun_failed_jobs(&self, issue: &IssueUrl, workflow_run: u64) -> Result<()> {
        self.gh(&[
            "run",
            "rerun",
            &workflow_run.to_string(),
            "--failed",
            "--repo",
            &issue.repo_slug(),
        ])
    }

    /// Run `gh <args>`, failing with its stderr if it exits non-zero.
    fn gh(&self, args: &[&str]) -> Result<()> {
        self.gh_stdout(args).map(drop)
    }

    /// Run `gh <args>` and return its trimmed stdout, failing with its stderr if
    /// it exits non-zero.
    fn gh_stdout(&self, args: &[&str]) -> Result<String> {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(CommandFailure::rejected(
                format!("gh {}", args.join(" ")),
                &output
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Every item of the `field` list of the GitHub API's `path`, across all its
    /// pages.
    fn gh_api_items(&self, path: &str, field: &str) -> Result<Vec<Value>> {
        self.retry_read(&format!("gh api {path}"), || {
            self.gh_api_items_once(path, field)
        })
    }

    fn gh_api_items_once(&self, path: &str, field: &str) -> Result<Vec<Value>> {
        let output = self.output(&["api", "--paginate", "--jq", &format!(".{field}[]"), path])?;
        if !output.status.success() {
            bail!(check_read::ReadFailure {
                command: format!("gh api {path}"),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                private_status: None,
            });
        }
        serde_json::Deserializer::from_slice(&output.stdout)
            .into_iter()
            .collect::<Result<_, _>>()
            .with_context(|| CommandFailure::InvalidJson {
                command: format!("gh api {path}"),
            })
    }

    /// Read-only JSON commands get three attempts, separated by one then two
    /// seconds, for transient failures only. Mutations use `gh_stdout` or
    /// `output_with_input` instead.
    fn gh_json(&self, args: &[&str]) -> Result<Value> {
        let command = format!("gh {}", args[..2].join(" "));
        self.retry_read(&command, || self.gh_json_once(args, &command))
    }

    fn retry_read<T>(&self, command: &str, mut read: impl FnMut() -> Result<T>) -> Result<T> {
        for attempt in 1..=3 {
            let error = match read() {
                Ok(value) => return Ok(value),
                Err(error) => error,
            };
            if attempt == 3 || !check_read::is_transient(&error) {
                return Err(error);
            }
            crate::progress::step(format!(
                "{command} failed (attempt {attempt}/3): {error:#}; retrying in {attempt}s"
            ));
            self.interruption.pause(Duration::from_secs(attempt))?;
        }
        unreachable!("the third attempt returns success or the last cause")
    }

    fn gh_json_once(&self, args: &[&str], command: &str) -> Result<Value> {
        let output = self.output(args)?;
        if !output.status.success() {
            bail!(check_read::ReadFailure {
                command: command.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                private_status: None,
            });
        }
        serde_json::from_slice(&output.stdout).with_context(|| CommandFailure::InvalidJson {
            command: command.to_string(),
        })
    }
    fn output(&self, args: &[&str]) -> Result<Output> {
        self.output_with_input(args, None)
    }

    fn output_with_input(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output> {
        process::output(
            Command::new("gh").args(args),
            input,
            Control {
                name: "gh",
                interruption: self.interruption,
                stop: &|child| process::stop(child, &[libc::SIGTERM]),
            },
        )
        .map_err(|error| {
            self.interruption
                .check()
                .err()
                .unwrap_or_else(|| error.context(CommandFailure::Execution))
        })
    }
}

/// Whether the issue `json` describes, with its `state`, is open.
fn state_is_open(json: &Value) -> Result<bool> {
    Ok(json["state"]
        .as_str()
        .context(CommandFailure::InvalidResponse(
            "gh output has no state".into(),
        ))?
        == "OPEN")
}

/// An issue as an Architect run reads the issue its Architecture review
/// ended on, and as a Claim reads its issue after a Self-merge.
#[derive(Clone)]
pub struct Issue {
    pub title: Option<String>,
    pub is_open: bool,
    pub labels: Labels,
    pub created: DateTime<Utc>,
}

/// The labels of the issue `json` describes, with its `labels`.
fn labels_of(json: &Value) -> Result<Labels> {
    Ok(named(json["labels"].as_array().context(
        CommandFailure::InvalidResponse("gh output has no labels".into()),
    )?))
}

/// The labels `labels` describe, each with its `name`.
fn named(labels: &[Value]) -> Labels {
    labels
        .iter()
        .filter_map(|label| label["name"].as_str())
        .collect()
}

/// The REST API's path for the labels of `issue`.
fn labels_path(issue: &IssueUrl) -> String {
    format!("repos/{}/issues/{}/labels", issue.repo_slug(), issue.number)
}

/// The REST API's path for `label` on `issue`.
fn label_path(issue: &IssueUrl, label: &str) -> String {
    format!("{}/{label}", labels_path(issue))
}

/// The command that does what [`GitHub::remove_label`] does, to run by hand.
pub fn remove_label_command(issue: &IssueUrl, label: Label) -> String {
    format!("gh api --method DELETE {}", label_path(issue, label.name()))
}

/// The command that adds `label` to `issue`, keeping its other labels, to
/// run by hand: unlike [`GitHub::set_labels`], it needs none of the others named.
pub fn add_label_command(issue: &IssueUrl, label: Label) -> String {
    format!(
        "gh api --method POST {} -f 'labels[]={label}'",
        labels_path(issue)
    )
}

/// A Spec's sub-issue, as a Spec run reads it.
#[derive(Clone)]
pub struct Ticket {
    pub issue: IssueUrl,
    pub title: Option<String>,
    pub is_open: bool,
    pub labels: Labels,
    /// It has sub-issues of its own.
    pub has_sub_issues: bool,
    /// The issues it is blocked by, in the Spec or not.
    pub blockers: Vec<IssueUrl>,
    /// Those blockers that are open.
    pub open_blockers: Vec<TicketBlocker>,
}

/// An open blocker and the reporting facts read with the Ticket graph.
#[derive(Clone)]
pub struct TicketBlocker {
    pub issue: IssueUrl,
    pub title: Option<String>,
    pub labels: Labels,
}

#[cfg(test)]
impl From<IssueUrl> for TicketBlocker {
    fn from(issue: IssueUrl) -> Self {
        Self {
            issue,
            title: None,
            labels: Labels::default(),
        }
    }
}

/// Every sub-issue of `issue`, open or closed, with its labels, whether it
/// has sub-issues of its own and the issues it is blocked by: its GitHub
/// "blocked by" links, never the text of its body.
/// Forty labels per blocker keep the nested query at 420,100 possible nodes,
/// below GitHub's 500,000-node limit; overflowing labels are read separately.
const TICKETS_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      subIssues(first: 100) {
        nodes {
          url
          title
          state
          labels(first: 100) { nodes { name } }
          subIssues { totalCount }
          blockedBy(first: 100) { nodes { url title state labels(first: 40) { nodes { name } pageInfo { hasNextPage } } } }
        }
      }
    }
  }
}";

/// The `nodes` of `connection`, the `what` of a GraphQL answer.
fn nodes<'a>(connection: &'a Value, what: &str) -> Result<&'a Vec<Value>> {
    connection["nodes"].as_array().with_context(|| {
        CommandFailure::InvalidResponse(format!("gh api graphql output has no {what}"))
    })
}

/// Whether the issue a GraphQL answer's `node` describes is open.
fn node_is_open(node: &Value) -> Result<bool> {
    Ok(node["state"]
        .as_str()
        .context(CommandFailure::InvalidResponse(
            "gh api graphql output has an issue with no state".into(),
        ))?
        == "OPEN")
}

/// The full Issue URL GitHub returned for a Ticket or blocker.
fn node_issue(node: &Value) -> Result<IssueUrl> {
    IssueUrl::parse(
        node["url"]
            .as_str()
            .context(CommandFailure::InvalidResponse(
                "gh api graphql output has an issue with no URL".into(),
            ))?,
    )
    .context(CommandFailure::InvalidResponse(
        "gh api graphql output has an invalid issue URL".into(),
    ))
}

/// The number of the issue a GraphQL answer's `node` describes.
fn node_number(node: &Value) -> Result<u64> {
    node["number"]
        .as_u64()
        .context(CommandFailure::InvalidResponse(
            "gh api graphql output has an issue with no number".into(),
        ))
}

/// How many sub-issues the issue a GraphQL answer's `node` describes has.
fn sub_issue_count(node: &Value) -> Result<u64> {
    node["subIssues"]["totalCount"]
        .as_u64()
        .context(CommandFailure::InvalidResponse(
            "gh api graphql output has an issue with no subIssues count".into(),
        ))
}

/// An issue as a listing of its repository's issues gives it.
#[derive(Debug, Clone)]
pub struct ListedIssue {
    pub issue: IssueUrl,
    pub title: String,
    pub labels: Labels,
}

/// An open issue labelled for a Pickup run to take, as the Pickup run reads
/// what its listing doesn't give.
pub struct Candidate {
    /// The issue it is a sub-issue of, if it is one.
    pub parent: Option<IssueUrl>,
    /// For each of its sub-issues, whether that one is open.
    pub sub_issue_is_open: Vec<bool>,
    /// The numbers of the open issues it is blocked by.
    pub open_blockers: Vec<u64>,
    /// The latest of the changes that shape it, if its timeline has one.
    pub last_shaped: Option<Shaped>,
    /// Complete label history records the last Claim ending, with a ready-label
    /// application at that timestamp or later, neither withdrawn nor superseded.
    pub handed_back: bool,
    /// Number of Claim applications in complete timeline order. Detects a
    /// Claim made since selection, even if it ended in the same second.
    pub claim_generation: usize,
    /// A Claim is active at the history sample, even if a prior issue listing
    /// still showed only `ready-for-agent`.
    pub claimed: bool,
}

impl Candidate {
    /// It has sub-issues of its own.
    pub fn has_sub_issues(&self) -> bool {
        !self.sub_issue_is_open.is_empty()
    }
}

/// The last Claim and whether the Day shift handed it back. Claim events at a
/// timestamp are folded before ready labels, retaining order within each label.
enum ClaimHistory {
    NeverClaimed,
    Claimed,
    Released,
    HandedBack,
}

impl ClaimHistory {
    fn after(self, name: Option<&str>, applied: bool, ready: Label) -> Self {
        if name.is_some_and(|name| crate::claim::IN_PROGRESS.is_named(name)) {
            return match (self, applied) {
                (_, true) => Self::Claimed,
                (Self::Claimed, false) => Self::Released,
                (state, false) => state,
            };
        }
        if name.is_some_and(|name| ready.is_named(name)) {
            return match (self, applied) {
                (Self::Released | Self::HandedBack, true) => Self::HandedBack,
                (Self::HandedBack, false) => Self::Released,
                (state, _) => state,
            };
        }
        self
    }
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

/// An issue's parent, the state of each of its sub-issues, the issues it is
/// blocked by, and the events of its timeline that shape it: each time a
/// label was applied or removed (paginated to completion), and the last
/// sub-issue or "blocked by" link added or removed. They are read from the
/// timeline because adding a sub-issue does not change the issue's update
/// time. The sub-issues read are the first hundred, the ones
/// [`TICKETS_QUERY`] reads, so a Pickup run and a Spec run see the same
/// Tickets.
const CANDIDATE_QUERY: &str = "\
query($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    issue(number: $number) {
      parent { url }
      subIssues(first: 100) { nodes { state } }
      blockedBy(first: 100) { nodes { number state } }
      labelled: timelineItems(first: 100, after: $after, itemTypes: [LABELED_EVENT, UNLABELED_EVENT]) {
        pageInfo { hasNextPage endCursor }
        nodes {
          __typename
          ... on LabeledEvent { createdAt label { name } }
          ... on UnlabeledEvent { createdAt label { name } }
        }
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

const PR_FIELDS: &str = "number,url,state,headRefName,baseRefName,isDraft";

const SNAPSHOT_FIELDS: &str =
    "number,url,state,headRefName,baseRefName,isDraft,mergeable,headRefOid,isCrossRepository";

/// The fields needed for one pull request observation, read together.
#[derive(Debug, Clone)]
pub struct PrSnapshot {
    pub pr: PullRequest,
    pub mergeable: Mergeable,
    pub head_commit: String,
    pub from_fork: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
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
            json[name].as_str().map(String::from).with_context(|| {
                CommandFailure::InvalidResponse(format!("gh output has no {name}"))
            })
        };
        let state = match field("state")?.as_str() {
            "OPEN" => PrState::Open,
            "CLOSED" => PrState::Closed,
            "MERGED" => PrState::Merged,
            other => bail!(CommandFailure::InvalidResponse(format!(
                "gh output has an unknown PR state {other}"
            ))),
        };
        Ok(PullRequest {
            number: json["number"]
                .as_u64()
                .context(CommandFailure::InvalidResponse(
                    "gh output has no number".into(),
                ))?,
            url: field("url")?,
            state,
            head: field("headRefName")?,
            base: field("baseRefName")?,
            is_draft: json["isDraft"]
                .as_bool()
                .context(CommandFailure::InvalidResponse(
                    "gh output has no isDraft".into(),
                ))?,
        })
    }
}

/// The two subjects the Inbox reads, owning their GraphQL selection and decoding.
#[derive(Clone, Copy)]
enum InboxSubject {
    Issue,
    PullRequest,
}

impl InboxSubject {
    fn selection(self) -> (&'static str, &'static str) {
        match self {
            Self::Issue => (
                "issue",
                "state closedAt labels(first: 100) { nodes { name } }",
            ),
            Self::PullRequest => (
                "pullRequest",
                "url state mergedAt closedAt mergeable commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }",
            ),
        }
    }

    fn read(self, url: &str, json: &Value) -> Result<InboxState> {
        match self {
            Self::Issue => IssueState::from_json(url, json).map(InboxState::Issue),
            Self::PullRequest => {
                PullRequestState::from_json(url, json).map(InboxState::PullRequest)
            }
        }
    }
}

/// The Inbox's facts for either kind of GitHub subject. Untagged to keep
/// reading the pull-request-only cache written before issue reads existed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum InboxState {
    Issue(IssueState),
    PullRequest(PullRequestState),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueState {
    pub closed: bool,
    pub closed_at: Option<String>,
    pub needs_triage: bool,
    /// The first kept answer showing needs-triage absent, reset if it returns.
    pub triaged_at: Option<String>,
}

impl IssueState {
    fn from_json(url: &str, json: &Value) -> Result<Self> {
        let closed = match json["state"].as_str() {
            Some("OPEN") => false,
            Some("CLOSED") => true,
            other => bail!(CommandFailure::InvalidResponse(format!(
                "gh output has an unknown issue state {other:?} for {url}"
            ))),
        };
        Ok(Self {
            closed,
            closed_at: json["closedAt"].as_str().map(String::from),
            needs_triage: json["labels"]["nodes"].as_array().is_some_and(|labels| {
                labels.iter().any(|label| {
                    label["name"]
                        .as_str()
                        .is_some_and(|name| crate::labels::NEEDS_TRIAGE.is_named(name))
                })
            }),
            triaged_at: None,
        })
    }
}

/// What GitHub last showed for one pull request, including head CI and mergeability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PullRequestState {
    pub state: PrState,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    #[serde(default)]
    pub ci: InboxCi,
    #[serde(default)]
    pub mergeable: InboxMergeable,
}

/// Mergeability is independent of head CI. Old or computing answers imply no conflict.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InboxMergeable {
    Mergeable,
    Conflicting,
    #[default]
    Unknown,
}

impl InboxMergeable {
    fn from_json(pr: &Value) -> Self {
        match pr["mergeable"].as_str() {
            Some("MERGEABLE") => Self::Mergeable,
            Some("CONFLICTING") => Self::Conflicting,
            _ => Self::Unknown,
        }
    }
}

/// The combined checks and commit status on the latest pull-request commit.
/// Old caches and missing or unfamiliar answers must never imply passing CI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InboxCi {
    Green,
    Red,
    Pending,
    NoChecks,
    #[default]
    Unknown,
}

impl InboxCi {
    fn from_json(pr: &Value) -> Self {
        let commit = &pr["commits"]["nodes"][0]["commit"];
        match commit["statusCheckRollup"]["state"].as_str() {
            Some("SUCCESS") => Self::Green,
            Some("FAILURE" | "ERROR") => Self::Red,
            Some("PENDING" | "EXPECTED") => Self::Pending,
            None if commit.get("statusCheckRollup").is_some_and(Value::is_null) => Self::NoChecks,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Green => "CI green",
            Self::Red => "CI red",
            Self::Pending => "CI pending",
            Self::NoChecks => "No CI checks",
            Self::Unknown => "CI unknown",
        }
    }
}

impl PullRequestState {
    fn from_json(url: &str, json: &Value) -> Result<Self> {
        let state = match json["state"].as_str() {
            Some("OPEN") => PrState::Open,
            Some("CLOSED") => PrState::Closed,
            Some("MERGED") => PrState::Merged,
            other => bail!(CommandFailure::InvalidResponse(format!(
                "gh output has an unknown PR state {other:?} for {url}"
            ))),
        };
        let at = |name: &str| json[name].as_str().map(String::from);
        Ok(PullRequestState {
            state,
            merged_at: at("mergedAt"),
            closed_at: at("closedAt"),
            ci: InboxCi::from_json(json),
            mergeable: InboxMergeable::from_json(json),
        })
    }
}

/// A GitHub owner or repository name safe to interpolate into GraphQL or a
/// `gh api` path: one letter or digit, then only letters, digits, `.`, `_`
/// and `-`, at most 100 bytes.
pub(crate) fn github_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && name.len() <= 100
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// `owner/repo`, with both sides a [`github_name`] and no extra slash.
pub(crate) fn valid_repo_slug(slug: &str) -> bool {
    let Some((owner, repo)) = slug.split_once('/') else {
        return false;
    };
    !repo.contains('/') && github_name(owner) && github_name(repo)
}

/// The owner, repository and pull request number of a
/// `https://github.com/<owner>/<repo>/pull/<number>` URL, if it parses.
fn split_pull_request_url(url: &str) -> Option<(String, String, u64)> {
    let path = url.strip_prefix("https://github.com/")?;
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [owner, repo, "pull", number] if !owner.is_empty() && !repo.is_empty() => number
            .parse()
            .ok()
            .map(|number| (owner.to_string(), repo.to_string(), number)),
        _ => None,
    }
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

/// What GitHub said of a check run or commit status: a check run's
/// conclusion, or that it has not completed, or a commit status's state,
/// each named as GitHub names it. What each means for CI is CI watch's to
/// read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conclusion {
    /// A check run that has not completed.
    Unfinished,
    Success,
    Failure,
    Neutral,
    Skipped,
    Cancelled,
    TimedOut,
    ActionRequired,
    Stale,
    StartupFailure,
    /// A commit status still pending.
    Pending,
    /// A commit status in error.
    Error,
    /// A conclusion or state GitHub does not document, as GitHub wrote it:
    /// empty when a completed check run has none.
    Unrecognised(String),
}

impl Conclusion {
    /// The conclusion `conclusion` of a completed check run.
    fn of_check_run(conclusion: Option<&str>) -> Self {
        match conclusion {
            Some("success") => Self::Success,
            Some("failure") => Self::Failure,
            Some("neutral") => Self::Neutral,
            Some("skipped") => Self::Skipped,
            Some("cancelled") => Self::Cancelled,
            Some("timed_out") => Self::TimedOut,
            Some("action_required") => Self::ActionRequired,
            Some("stale") => Self::Stale,
            Some("startup_failure") => Self::StartupFailure,
            other => Self::Unrecognised(other.unwrap_or_default().to_string()),
        }
    }

    /// The state `state` of a commit status.
    fn of_commit_status(state: Option<&str>) -> Self {
        match state {
            Some("success") => Self::Success,
            Some("failure") => Self::Failure,
            Some("pending") => Self::Pending,
            Some("error") => Self::Error,
            other => Self::Unrecognised(other.unwrap_or_default().to_string()),
        }
    }
}

/// A check run or a commit status.
#[derive(Clone)]
pub struct Check {
    pub name: String,
    pub conclusion: Conclusion,
    pub url: Option<String>,
    /// The GitHub Actions job it is, if it is one. No other check can be
    /// re-run: not a commit status, nor a check run of another app.
    pub job: Option<ActionsJob>,
    /// Workflow identity, including for a run whose jobs are not listed yet.
    pub workflow_run: Option<WorkflowRun>,
}

/// A run's identity within its workflow. A new attempt keeps both ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowRun {
    pub id: u64,
    pub workflow_id: u64,
}

#[derive(serde::Deserialize)]
struct ListedRun {
    id: u64,
    workflow_id: u64,
    created_at: String,
    status: Option<String>,
    conclusion: Option<String>,
    name: Option<String>,
    html_url: Option<String>,
    #[serde(skip)]
    superseded: bool,
}

struct CheckRuns {
    records: Vec<Value>,
    workflows: Option<Vec<ListedRun>>,
}

/// A job of a GitHub Actions workflow run, as one of its attempts ran it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionsJob {
    /// Its check run's id, which a new attempt of the job does not keep.
    pub check_run: u64,
    /// The workflow run it belongs to.
    pub workflow_run: u64,
    /// Whether it is an Abandoned job, which only the CI watch tells, and
    /// only of a job that failed: false until then.
    pub abandoned: bool,
}

/// A GitHub Actions job as the jobs API reports it, with no annotation or
/// log text: its conclusion and each of its steps', in order, a step not
/// completed being [`Conclusion::Unfinished`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    pub conclusion: Conclusion,
    pub steps: Vec<Conclusion>,
}

impl ActionsJob {
    /// The job whose check run is `check_run`, if `details_url`, the check
    /// run's, is a job's page: `…/actions/runs/<workflow run>/job/<job>`.
    fn of(check_run: Option<u64>, details_url: Option<&str>) -> Option<Self> {
        let (_, after) = details_url?.split_once("/actions/runs/")?;
        let (workflow_run, _) = after.split_once("/job/")?;
        Some(ActionsJob {
            check_run: check_run?,
            workflow_run: workflow_run.parse().ok()?,
            abandoned: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_check_run_whose_details_are_a_jobs_page_is_an_actions_job() {
        let job = "https://github.com/acme/widgets/actions/runs/900/job/41";
        assert_eq!(
            ActionsJob::of(Some(41), Some(job)),
            Some(ActionsJob {
                check_run: 41,
                workflow_run: 900,
                abandoned: false,
            })
        );
        for not_a_job in [
            "https://ci.example/test",
            "https://github.com/acme/widgets/actions/runs/900",
            "https://github.com/acme/widgets/actions/runs/latest/job/41",
            "https://github.com/acme/widgets/runs/41",
        ] {
            assert_eq!(ActionsJob::of(Some(41), Some(not_a_job)), None);
        }
        assert_eq!(ActionsJob::of(Some(41), None), None);
        assert_eq!(ActionsJob::of(None, Some(job)), None);
    }
}
