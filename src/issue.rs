//! The Issue URL, the repository `origin` names, and the Origin match.

use anyhow::{Result, bail};

/// A parsed `https://github.com/<owner>/<repo>/issues/<n>`.
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Issue `number` in the same repository.
    pub fn sibling(&self, number: u64) -> IssueUrl {
        IssueUrl {
            url: format!("https://github.com/{}/issues/{number}", self.repo_slug()),
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            number,
        }
    }

    /// `owner/repo`, as `gh --repo` takes it.
    pub fn repo_slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The Origin match: does `origin_url` (the raw configured origin, HTTPS or
    /// SSH, with or without `.git`) name this issue's repository? Case-insensitive.
    pub fn matches_origin(&self, origin_url: &str) -> bool {
        Repo::of_origin(origin_url).is_some_and(|origin| {
            origin.owner.eq_ignore_ascii_case(&self.owner)
                && origin.name.eq_ignore_ascii_case(&self.repo)
        })
    }
}

/// The GitHub repository an `origin` remote names, spelled as its URL spells
/// it.
#[derive(Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// The repository `origin_url` names (the raw configured origin, HTTPS or
    /// SSH, with or without `.git`), or `None` for a non-GitHub origin.
    pub fn of_origin(origin_url: &str) -> Option<Self> {
        const GIT: &str = ".git";
        let url = origin_url.trim();
        let path = [
            "https://github.com/",
            "git@github.com:",
            "ssh://git@github.com/",
        ]
        .into_iter()
        .find_map(|prefix| {
            let start = url.get(..prefix.len())?;
            start
                .eq_ignore_ascii_case(prefix)
                .then(|| &url[prefix.len()..])
        })?;
        let path = path.trim_end_matches('/');
        let path = match path
            .len()
            .checked_sub(GIT.len())
            .and_then(|at| path.get(at..))
        {
            Some(end) if end.eq_ignore_ascii_case(GIT) => &path[..path.len() - GIT.len()],
            _ => path,
        };
        match path.split('/').collect::<Vec<_>>().as_slice() {
            [owner, name] if !owner.is_empty() && !name.is_empty() => Some(Repo {
                owner: owner.to_string(),
                name: name.to_string(),
            }),
            _ => None,
        }
    }

    /// `owner/name`, as its URL spells it.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_of_an_origin_keeps_its_spelling_whatever_the_form_of_the_url() {
        let repo = |owner: &str, name: &str| {
            Some(Repo {
                owner: owner.to_string(),
                name: name.to_string(),
            })
        };
        for (origin, expected) in [
            (
                "https://github.com/acme/widgets.git",
                repo("acme", "widgets"),
            ),
            ("https://github.com/Acme/Widgets", repo("Acme", "Widgets")),
            (
                "HTTPS://GitHub.com/acme/widgets.GIT/",
                repo("acme", "widgets"),
            ),
            ("git@github.com:acme/widgets.git\n", repo("acme", "widgets")),
            ("ssh://git@github.com/acme/widgets", repo("acme", "widgets")),
            ("https://gitlab.com/acme/widgets.git", None),
            ("https://github.com/acme", None),
            ("https://github.com/acme/widgets/extra", None),
            ("../origin.git", None),
        ] {
            assert_eq!(Repo::of_origin(origin), expected, "{origin}");
        }
    }
}
