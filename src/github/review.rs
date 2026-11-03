//! Pull request eligibility and the evidence handed to a Review session.

use anyhow::{Context, Result, ensure};

use super::GitHub;
use crate::review::PullRequestUrl;
use crate::trust::trusted_association;

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
}

impl GitHub {
    /// Fail closed on missing metadata, before creating a worktree or session.
    pub fn review_pull_request(&self, pr: &PullRequestUrl) -> Result<ReviewPullRequest> {
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
        ensure!(
            !node["draft"]
                .as_bool()
                .context("pull request has no draft state")?,
            "pull request {} is a draft",
            pr.url
        );
        let head_repo = text(&node["head"]["repo"]["full_name"])?;
        let base_repo = text(&node["base"]["repo"]["full_name"])?;
        ensure!(
            head_repo.eq_ignore_ascii_case(&base_repo)
                && base_repo.eq_ignore_ascii_case(&pr.issue.repo_slug()),
            "pull request {} is from a fork",
            pr.url
        );
        ensure!(
            trusted_association(&text(&node["author_association"])?),
            "pull request {} author is not a Trusted account",
            pr.url
        );
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
        let mut after = None;
        let mut issues = std::collections::BTreeSet::new();
        loop {
            let json = self.issue_query(&pr.issue, CLOSING_ISSUES, after.as_deref())?;
            let page = &json["data"]["repository"]["pullRequest"]["closingIssuesReferences"];
            for node in page["nodes"]
                .as_array()
                .context("closing issues have no nodes")?
            {
                let url = node["url"].as_str().context("closing issue has no URL")?;
                if issues.insert(url.to_owned()) {
                    let issue = crate::issue::IssueUrl::parse(url)?;
                    markdown.push_str("\n## Linked closing issue (Spec)\n\n");
                    markdown.push_str(&self.issue_for_session(&issue)?);
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
            body_alone: issues.is_empty(),
        })
    }
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
