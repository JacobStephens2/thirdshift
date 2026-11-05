//! A Review run starts on a pull request, independently of any issue branch.

use anyhow::{Result, bail, ensure};

use crate::issue::{IssueUrl, Repo};
use std::num::NonZeroU64;

mod reservation;
pub(crate) use reservation::Reservation;

pub(crate) enum Target {
    PullRequest(PullRequestUrl),
    Author(String),
}

/// A validated PR argument before shorthand is resolved against origin.
pub(crate) enum PullRequestInput {
    Url(PullRequestUrl),
    Number(NonZeroU64),
}

impl PullRequestInput {
    pub fn parse(input: &str) -> Result<Self> {
        if !input.is_empty() && input.bytes().all(|byte| byte.is_ascii_digit()) {
            let number = input.parse().map_err(|_| {
                anyhow::anyhow!("pull request number must be positive and fit in u64")
            })?;
            Ok(Self::Number(number))
        } else {
            PullRequestUrl::parse(input).map(Self::Url)
        }
    }

    pub fn resolve(&self, repo: &Repo) -> Result<PullRequestUrl> {
        match self {
            Self::Url(pr) => Ok(pr.clone()),
            Self::Number(number) => {
                PullRequestUrl::parse(&format!("https://github.com/{}/pull/{number}", repo.slug()))
            }
        }
    }
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
