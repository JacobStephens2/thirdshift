//! Pull request eligibility and the evidence handed to a Review session.

use anyhow::{Context, Result, ensure};

use super::GitHub;
use crate::review::PullRequestUrl;
use crate::trust::trusted_association;

#[derive(Clone)]
pub(crate) struct ReviewPullRequest {
    pub title: String,
    pub body: String,
    pub head: String,
    pub branch: String,
    pub base: String,
    pub base_head: String,
}

pub(crate) struct ReviewInput {
    pub markdown: String,
    pub body_alone: bool,
    /// Closing issues whose title and body were copied into `markdown`.
    pub closing_issues: Vec<crate::issue::IssueUrl>,
}

#[derive(serde::Deserialize)]
struct ReviewMarker {
    reviewed_head: Option<String>,
    pushed_head: Option<String>,
    authored_head: Option<String>,
}

impl ReviewMarker {
    fn reviewed(&self, head: &str) -> bool {
        self.reviewed_head.as_deref() == Some(head) || self.pushed_head.as_deref() == Some(head)
    }
}

impl GitHub {
    /// Enumerate every open PR, then order eligible Fix candidates locally.
    /// Mergeability and head CI are read separately, never from a search cache.
    pub fn fix_candidates(
        &self,
        repo: &crate::issue::Repo,
        base: &str,
        author: Option<&str>,
    ) -> Result<Vec<PullRequestUrl>> {
        if let Some(author) = author {
            ensure!(
                self.trusted_login(repo, author)?,
                "{author} is not a Trusted account on {}",
                repo.slug()
            );
        }
        let nodes = self.gh_api_items(
            &format!("repos/{}/pulls?state=open&per_page=100", repo.slug()),
            "",
        )?;
        let mut candidates = Vec::new();
        for node in nodes {
            let pr = PullRequestUrl::parse(
                node["html_url"]
                    .as_str()
                    .context("pull request has no URL")?,
            )?;
            if fix_candidate(&pr, &node, base, author, false)? {
                candidates.push(pr);
            }
        }
        candidates.sort_by_key(|pr| pr.issue.number);
        Ok(candidates)
    }

    /// Recheck admission both before reserving and while our label is held,
    /// returning the eligible head branch from that same observation.
    pub fn fix_candidate_branch(
        &self,
        pr: &PullRequestUrl,
        base: &str,
        author: Option<&str>,
        reserved: bool,
    ) -> Result<Option<String>> {
        let node = self.gh_json(&[
            "api",
            &format!("repos/{}/pulls/{}", pr.issue.repo_slug(), pr.issue.number),
        ])?;
        if !fix_candidate(pr, &node, base, author, reserved)? {
            return Ok(None);
        }
        let Some(snapshot) = self.pr_snapshot(&pr.issue, &pr.issue.number.to_string())? else {
            return Ok(None);
        };
        if snapshot.pr.state != super::PrState::Open
            || snapshot.pr.is_draft
            || snapshot.from_fork
            || snapshot.pr.base != base
            || node["head"]["sha"].as_str() != Some(&snapshot.head_commit)
        {
            return Ok(None);
        }
        if snapshot.mergeable == super::Mergeable::No {
            return Ok(Some(snapshot.pr.head));
        }
        Ok(
            crate::ci::needs_repair(&self.checks_on(&pr.issue, &snapshot.head_commit)?)
                .then_some(snapshot.pr.head),
        )
    }

    pub fn review_labels(&self, pr: &PullRequestUrl) -> Result<crate::labels::Labels> {
        let labels = self.gh_api_items(
            &format!(
                "repos/{}/issues/{}/labels?per_page=100",
                pr.issue.repo_slug(),
                pr.issue.number
            ),
            "",
        )?;
        Ok(super::named(&labels))
    }

    /// Page the repository rather than a capped search, then select by author
    /// and issue number locally so every waiting pull request is considered.
    pub fn waiting_review_pull_requests(
        &self,
        repo: &crate::issue::Repo,
        author: &str,
        reserved: Option<u64>,
    ) -> Result<Vec<PullRequestUrl>> {
        let nodes = self.gh_api_items(
            &format!("repos/{}/pulls?state=open&per_page=100", repo.slug()),
            "",
        )?;
        let mut candidates = Vec::new();
        for node in &nodes {
            if node["user"]["login"]
                .as_str()
                .is_none_or(|login| !login.eq_ignore_ascii_case(author))
                || (!(reserved.is_some() && reserved == node["number"].as_u64())
                    && super::labels_of(node)?.has(crate::claim::IN_PROGRESS))
            {
                continue;
            }
            let pr = PullRequestUrl::parse(
                node["html_url"]
                    .as_str()
                    .context("pull request has no URL")?,
            )?;
            if ensure_reviewable(&pr, node).is_err() {
                continue;
            }
            if self.reviewed_head(
                &pr,
                node["head"]["sha"]
                    .as_str()
                    .context("pull request has no head")?,
            )? {
                continue;
            }
            let mut base_waiting = false;
            for base_pr in nodes.iter().filter(|base| {
                base["head"]["ref"] == node["base"]["ref"]
                    && base["head"]["repo"]["full_name"] == node["base"]["repo"]["full_name"]
            }) {
                let base = PullRequestUrl::parse(
                    base_pr["html_url"]
                        .as_str()
                        .context("base pull request has no URL")?,
                )?;
                if !self.reviewed_head(
                    &base,
                    base_pr["head"]["sha"]
                        .as_str()
                        .context("base pull request has no head")?,
                )? {
                    base_waiting = true;
                    break;
                }
            }
            if base_waiting {
                continue;
            }
            candidates.push(pr);
        }
        candidates.sort_by_key(|pr| pr.issue.number);
        Ok(candidates)
    }

    pub(crate) fn reviewed_head(&self, pr: &PullRequestUrl, head: &str) -> Result<bool> {
        Ok(self
            .review_markers(pr)?
            .iter()
            .any(|marker| marker.reviewed(head)))
    }

    /// Authored Delivery evidence permits Fix Self-merge without claiming that
    /// a Review run has already reviewed the pull request.
    pub(crate) fn authored_or_reviewed_head(
        &self,
        pr: &PullRequestUrl,
        head: &str,
    ) -> Result<bool> {
        Ok(self
            .review_markers(pr)?
            .iter()
            .any(|marker| marker.reviewed(head) || marker.authored_head.as_deref() == Some(head)))
    }

    /// Fix commits delivered to the open pull request that owns this base.
    pub fn review_base_fixes(
        &self,
        pr: &PullRequestUrl,
        snapshot: &ReviewPullRequest,
    ) -> Result<Vec<String>> {
        let nodes = self.gh_api_items(
            &format!(
                "repos/{}/pulls?state=open&per_page=100",
                pr.issue.repo_slug()
            ),
            "",
        )?;
        let mut fixes = Vec::new();
        for node in nodes {
            if node["head"]["ref"].as_str() != Some(&snapshot.base)
                || node["head"]["repo"]["full_name"]
                    .as_str()
                    .is_none_or(|repo| !repo.eq_ignore_ascii_case(&pr.issue.repo_slug()))
            {
                continue;
            }
            let base = PullRequestUrl::parse(
                node["html_url"]
                    .as_str()
                    .context("base pull request has no URL")?,
            )?;
            for marker in self.review_markers(&base)? {
                if let (Some(reviewed), Some(pushed)) = (marker.reviewed_head, marker.pushed_head)
                    && reviewed != pushed
                {
                    fixes.push(pushed);
                }
            }
        }
        Ok(fixes)
    }

    fn review_markers(&self, pr: &PullRequestUrl) -> Result<Vec<ReviewMarker>> {
        let actor = self.gh_json(&["api", "user"])?;
        let actor = actor["login"]
            .as_str()
            .filter(|login| !login.is_empty())
            .context("authenticated GitHub account has no login")?;
        let comments = self.gh_api_items(
            &format!(
                "repos/{}/issues/{}/comments?per_page=100",
                pr.issue.repo_slug(),
                pr.issue.number
            ),
            "",
        )?;
        let mut markers = Vec::new();
        for comment in comments {
            if comment["user"]["login"]
                .as_str()
                .is_none_or(|login| !login.eq_ignore_ascii_case(actor))
            {
                continue;
            }
            // Delivery appends one marker after the session report. The report
            // is attacker-influenced text, so an earlier marker is not a record.
            if let Some(marker) = trailing_review_marker(comment["body"].as_str().unwrap_or("")) {
                markers.push(marker);
            }
        }
        Ok(markers)
    }

    /// Fail closed on missing metadata, before creating a worktree or session.
    pub fn review_pull_request(&self, pr: &PullRequestUrl) -> Result<ReviewPullRequest> {
        self.pull_request_for_work(pr, false)
    }

    /// Fix runs also accept drafts, sharing every other eligibility check.
    pub fn fix_pull_request(&self, pr: &PullRequestUrl) -> Result<ReviewPullRequest> {
        self.pull_request_for_work(pr, true)
    }

    fn pull_request_for_work(
        &self,
        pr: &PullRequestUrl,
        allow_draft: bool,
    ) -> Result<ReviewPullRequest> {
        let node = self.gh_json(&[
            "api",
            &format!("repos/{}/pulls/{}", pr.issue.repo_slug(), pr.issue.number),
        ])?;
        let text = |value: &serde_json::Value| -> Result<String> {
            value
                .as_str()
                .map(str::to_owned)
                .context("pull request metadata is missing text")
        };
        ensure!(
            !node["merged"]
                .as_bool()
                .context("pull request has no merged state")?,
            "pull request {} is merged",
            pr.url
        );
        ensure!(
            node["state"].as_str() == Some("open"),
            "pull request {} is closed",
            pr.url
        );
        if allow_draft {
            node["draft"]
                .as_bool()
                .context("pull request has no draft state")?;
            ensure_owned_and_trusted(pr, &node)?;
        } else {
            ensure_reviewable(pr, &node)?;
        }
        Ok(ReviewPullRequest {
            title: text(&node["title"])?,
            body: node["body"].as_str().unwrap_or("").into(),
            head: text(&node["head"]["sha"])?,
            branch: text(&node["head"]["ref"])?,
            base: text(&node["base"]["ref"])?,
            base_head: text(&node["base"]["sha"])?,
        })
    }

    pub fn comment_review(&self, pr: &PullRequestUrl, text: &str) -> Result<()> {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(text.as_bytes())?;
        self.gh(&[
            "pr",
            "comment",
            &pr.url,
            "--body-file",
            file.path().to_str().context("comment path is not UTF-8")?,
        ])
    }

    /// Read every closing issue without fetching its contents or PR comments.
    /// An incomplete observation must not authorize Review Pass admission.
    pub fn closing_issues(&self, pr: &PullRequestUrl) -> Result<Vec<crate::issue::IssueUrl>> {
        let mut after = None;
        let mut issues = std::collections::BTreeSet::new();
        let mut closing_issues = Vec::new();
        loop {
            let json = self.issue_query(&pr.issue, CLOSING_ISSUES, after.as_deref())?;
            let page = &json["data"]["repository"]["pullRequest"]["closingIssuesReferences"];
            for node in page["nodes"]
                .as_array()
                .context("closing issues have no nodes")?
            {
                let url = node["url"].as_str().context("closing issue has no URL")?;
                let issue = crate::issue::IssueUrl::parse(url)?;
                if issues.insert(issue.identity()) {
                    closing_issues.push(issue);
                }
            }
            let info = &page["pageInfo"];
            if !info["hasNextPage"]
                .as_bool()
                .context("closing issues have no hasNextPage")?
            {
                break;
            }
            let next = info["endCursor"]
                .as_str()
                .context("closing issues have no endCursor")?;
            ensure!(
                after.as_deref() != Some(next),
                "closing issues cursor did not advance"
            );
            after = Some(next.to_owned());
        }
        Ok(closing_issues)
    }

    /// Closing issues are paged independently of comments. Never silently
    /// fall back to the body when GitHub failed to supply the issue links.
    pub fn review_input(
        &self,
        pr: &PullRequestUrl,
        snapshot: &ReviewPullRequest,
    ) -> Result<ReviewInput> {
        let mut markdown = format!(
            "# {}\n\n{}\n\n## Pull request body\n\n{}\n",
            snapshot.title, pr.url, snapshot.body
        );
        let closing_issues = self.closing_issues(pr)?;
        for issue in &closing_issues {
            markdown.push_str("\n## Linked closing issue (Spec)\n\n");
            markdown.push_str(&self.issue_for_session(issue)?);
        }
        for (kind, suffix) in [
            ("issues", "comments"),
            ("pulls", "reviews"),
            ("pulls", "comments"),
        ] {
            let comments = self.gh_api_items(
                &format!(
                    "repos/{}/{kind}/{}/{suffix}?per_page=100",
                    pr.issue.repo_slug(),
                    pr.issue.number
                ),
                "",
            )?;
            for comment in comments {
                let association = comment["author_association"]
                    .as_str()
                    .context("PR comment has no author association")?;
                if let Some(login) = comment["user"]["login"]
                    .as_str()
                    .filter(|_| trusted_association(association))
                {
                    markdown.push_str(&format!(
                        "\n### Comment by @{login}\n\n{}\n",
                        comment["body"].as_str().unwrap_or("")
                    ));
                } else {
                    markdown.push_str("\n_A comment from an account that is not a Trusted account was left out._\n");
                }
            }
        }
        Ok(ReviewInput {
            markdown,
            body_alone: closing_issues.is_empty(),
            closing_issues,
        })
    }
}

/// The marker thirdshift appends is the comment's suffix. A marker earlier
/// in the session report must not decide which head was reviewed.
fn trailing_review_marker(body: &str) -> Option<ReviewMarker> {
    let body = body.trim_end();
    let start = body.rfind("<!-- thirdshift-review ")?;
    let after = &body[start + "<!-- thirdshift-review ".len()..];
    let (json, rest) = after.split_once("-->")?;
    if !rest.trim().is_empty() {
        return None;
    }
    serde_json::from_str(json.trim()).ok()
}

/// Shared by explicit runs (which explain refusal) and author selection
/// (which passes over ineligible pull requests).
fn ensure_reviewable(pr: &PullRequestUrl, node: &serde_json::Value) -> Result<()> {
    ensure!(
        !node["draft"]
            .as_bool()
            .context("pull request has no draft state")?,
        "pull request {} is a draft",
        pr.url
    );
    ensure_owned_and_trusted(pr, node)
}

fn fix_candidate(
    pr: &PullRequestUrl,
    node: &serde_json::Value,
    base: &str,
    author: Option<&str>,
    reserved: bool,
) -> Result<bool> {
    Ok(node["state"] == "open"
        && node["base"]["ref"].as_str() == Some(base)
        && author.is_none_or(|author| {
            node["user"]["login"]
                .as_str()
                .is_some_and(|login| login.eq_ignore_ascii_case(author))
        })
        && ensure_reviewable(pr, node).is_ok()
        && (reserved || !super::labels_of(node)?.has(crate::claim::IN_PROGRESS)))
}

fn ensure_owned_and_trusted(pr: &PullRequestUrl, node: &serde_json::Value) -> Result<()> {
    fn text(value: &serde_json::Value) -> Result<&str> {
        value
            .as_str()
            .context("pull request metadata is missing text")
    }
    let head_repo = text(&node["head"]["repo"]["full_name"])?;
    let base_repo = text(&node["base"]["repo"]["full_name"])?;
    ensure!(
        head_repo.eq_ignore_ascii_case(base_repo)
            && base_repo.eq_ignore_ascii_case(&pr.issue.repo_slug()),
        "pull request {} is from a fork",
        pr.url
    );
    ensure!(
        trusted_association(text(&node["author_association"])?),
        "pull request {} author is not a Trusted account",
        pr.url
    );
    Ok(())
}

const CLOSING_ISSUES: &str =
    "query ReviewClosingIssues($owner: String!, $repo: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      closingIssuesReferences(first: 100, after: $after) {
        nodes { url }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}";
