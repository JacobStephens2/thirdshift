//! A Review run starts on a pull request, independently of any issue branch.

use anyhow::{Result, bail, ensure};

use crate::issue::IssueUrl;

mod reservation;
pub(crate) use reservation::Reservation;

pub(crate) enum Target {
    PullRequest(PullRequestUrl),
    Author(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Address,
    RecommendOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequestUrl {
    pub url: String,
    /// GitHub's issue endpoints also identify pull requests by this number.
    pub issue: IssueUrl,
}

impl PullRequestUrl {
    pub fn parse(url: &str) -> Result<Self> {
        let Some((repo, number)) = url.trim_end_matches('/').split_once("/pull/") else {
            bail!("not a GitHub pull request URL: {url}");
        };
        let issue = IssueUrl::parse(&format!("{repo}/issues/{number}"))
            .map_err(|_| anyhow::anyhow!("not a GitHub pull request URL: {url}"))?;
        ensure!(issue.number > 0, "pull request number must be positive");
        Ok(Self {
            url: url.into(),
            issue,
        })
    }
}
