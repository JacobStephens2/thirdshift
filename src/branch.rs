//! Issue branch selection: start a fresh Issue branch, or continue the one
//! already on origin (ADR-0002).

use anyhow::{Result, bail};

use crate::git::Git;
use crate::github::{self, PullRequest};
use crate::issue::IssueUrl;

pub enum Selection {
    /// No Issue branch has been used yet: start `branch` from the Base branch.
    Fresh { branch: String },
    /// `branch` is on origin at `origin_sha`, with no PR or the open `pr`.
    Continuation {
        branch: String,
        origin_sha: String,
        pr: Option<PullRequest>,
    },
}

impl Selection {
    pub fn branch(&self) -> &str {
        match self {
            Selection::Fresh { branch } | Selection::Continuation { branch, .. } => branch,
        }
    }

    /// Where `branch` is on origin, if it is there.
    pub fn origin_sha(&self) -> Option<&str> {
        match self {
            Selection::Fresh { .. } => None,
            Selection::Continuation { origin_sha, .. } => Some(origin_sha),
        }
    }
}

/// Pick the Issue branch for `issue` from the Issue branches on origin (one
/// `git ls-remote`) and their PRs in any state (one `gh pr list`).
pub fn select(launch: &Git, issue: &IssueUrl) -> Result<Selection> {
    let first = format!("issue-{}", issue.number);
    let remote = launch.run(&[
        "ls-remote",
        "--heads",
        "origin",
        &format!("refs/heads/{first}"),
        &format!("refs/heads/{first}-branch-*"),
    ])?;
    let on_origin: Vec<(u64, String, String)> = remote
        .lines()
        .filter_map(|line| {
            let (sha, name) = line.split_once('\t')?;
            let branch = name.strip_prefix("refs/heads/")?;
            let number = branch_number(issue, branch)?;
            Some((number, branch.to_string(), sha.to_string()))
        })
        .collect();
    let prs: Vec<PullRequest> = github::pull_requests_from(issue, &first)?
        .into_iter()
        .filter(|pr| branch_number(issue, &pr.head).is_some())
        .collect();

    let highest = on_origin
        .iter()
        .map(|(number, ..)| *number)
        .chain(prs.iter().filter_map(|pr| branch_number(issue, &pr.head)))
        .max();
    let Some(highest) = highest else {
        return Ok(Selection::Fresh { branch: first });
    };
    let branch = branch_name(issue, highest);
    // A branch can have had several PRs; the newest one decides.
    let pr = prs
        .into_iter()
        .filter(|pr| pr.head == branch)
        .max_by_key(|pr| pr.number);
    let origin_sha = on_origin
        .into_iter()
        .find_map(|(number, _, sha)| (number == highest).then_some(sha));
    match (origin_sha, pr) {
        (Some(origin_sha), None) => Ok(Selection::Continuation {
            branch,
            origin_sha,
            pr: None,
        }),
        (Some(origin_sha), Some(pr)) if pr.state == "OPEN" => Ok(Selection::Continuation {
            branch,
            origin_sha,
            pr: Some(pr),
        }),
        (_, pr) => bail!(
            "{branch} can't be continued ({}), and numbered Issue branches are not supported yet",
            match pr {
                Some(pr) => format!("its PR {} is {}", pr.url, pr.state.to_lowercase()),
                None => "it is gone from origin".to_string(),
            }
        ),
    }
}

/// `issue-<n>` is branch 1, `issue-<n>-branch-<k>` is branch k for k ≥ 2.
fn branch_number(issue: &IssueUrl, branch: &str) -> Option<u64> {
    let rest = branch.strip_prefix(&format!("issue-{}", issue.number))?;
    if rest.is_empty() {
        return Some(1);
    }
    let k: u64 = rest.strip_prefix("-branch-")?.parse().ok()?;
    (k >= 2 && rest == format!("-branch-{k}")).then_some(k)
}

fn branch_name(issue: &IssueUrl, number: u64) -> String {
    match number {
        1 => format!("issue-{}", issue.number),
        k => format!("issue-{}-branch-{k}", issue.number),
    }
}
