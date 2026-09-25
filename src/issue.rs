//! The Issue URL and the Origin match.

use anyhow::{Result, bail};

/// A parsed `https://github.com/<owner>/<repo>/issues/<n>`.
#[derive(Debug, Clone)]
pub struct IssueUrl {
    pub url: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl IssueUrl {
    pub fn parse(url: &str) -> Result<Self> {
        let parsed = url
            .strip_prefix("https://github.com/")
            .map(|path| path.trim_end_matches('/').split('/').collect::<Vec<_>>())
            .and_then(|parts| match parts.as_slice() {
                [owner, repo, "issues", number] if !owner.is_empty() && !repo.is_empty() => {
                    number.parse().ok().map(|number| IssueUrl {
                        url: url.to_string(),
                        owner: owner.to_string(),
                        repo: repo.to_string(),
                        number,
                    })
                }
                _ => None,
            });
        match parsed {
            Some(issue) => Ok(issue),
            None => bail!("not a GitHub issue URL: {url}"),
        }
    }

    /// `owner/repo`, as `gh --repo` takes it.
    pub fn repo_slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The Origin match: does `origin_url` (the raw configured origin, HTTPS or
    /// SSH, with or without `.git`) name this issue's repository? Case-insensitive.
    pub fn matches_origin(&self, origin_url: &str) -> bool {
        normalise_origin(origin_url).is_some_and(|origin| {
            origin == format!("github.com/{}", self.repo_slug()).to_lowercase()
        })
    }
}

/// `github.com/<owner>/<repo>`, lowercased, or `None` for a non-GitHub origin.
fn normalise_origin(origin_url: &str) -> Option<String> {
    let lower = origin_url.trim().to_lowercase();
    let path = lower
        .strip_prefix("https://github.com/")
        .or_else(|| lower.strip_prefix("git@github.com:"))
        .or_else(|| lower.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some(format!("github.com/{path}"))
}
