//! Issue branch selection: start a fresh Issue branch, or continue the one
//! already on origin (ADR-0002).

use anyhow::{Context, Result, bail};

use crate::git::Git;
use crate::github::{self, PrState, PullRequest};
use crate::issue::IssueUrl;

pub enum Selection {
    /// No Issue branch has been used yet: start `branch` from the Base branch.
    Fresh { branch: String },
    /// `branch` is on origin, with no PR or the open `pr`.
    Continuation {
        branch: String,
        pr: Option<PullRequest>,
    },
}

impl Selection {
    pub fn branch(&self) -> &str {
        match self {
            Selection::Fresh { branch } | Selection::Continuation { branch, .. } => branch,
        }
    }

    /// The Base branch: the open PR's base in a Continuation that has one,
    /// otherwise `checked_out`, the branch checked out in the launch
    /// directory (`None` on a detached HEAD). Says so on stderr when the PR's
    /// base replaces a different checked-out branch.
    pub fn base_branch(&self, checked_out: Option<&str>) -> Result<String> {
        if let Selection::Continuation {
            branch,
            pr: Some(pr),
        } = self
        {
            if let Some(checked_out) = checked_out.filter(|&c| c != pr.base) {
                eprintln!(
                    "thirdshift: continuing {branch} and its PR {}, so the Base branch is {}, not the checked-out {checked_out}",
                    pr.url, pr.base
                );
            }
            return Ok(pr.base.clone());
        }
        checked_out
            .map(String::from)
            .context("HEAD is detached; check out the branch the work should be based on")
    }
}

/// Pick the Issue branch for `issue` from the Issue branches on origin (one
/// `git ls-remote`) and their PRs in any state (one `gh pr list`). Fails if a
/// local copy of the chosen branch in the launch repository differs from
/// origin's: the Run replaces it and deletes it at cleanup.
pub fn select(launch: &Git, issue: &IssueUrl) -> Result<Selection> {
    let first_branch = branch_name(issue, 1);
    let remote = launch.run(&[
        "ls-remote",
        "--heads",
        "origin",
        &format!("refs/heads/{first_branch}"),
        &format!("refs/heads/{first_branch}-branch-*"),
    ])?;
    let on_origin: Vec<(u64, String)> = remote
        .lines()
        .filter_map(|line| {
            let (sha, name) = line.split_once('\t')?;
            let number = branch_number(issue, name.strip_prefix("refs/heads/")?)?;
            Some((number, sha.to_string()))
        })
        .collect();
    let prs: Vec<(u64, PullRequest)> =
        github::pull_requests_with_head_prefix(issue, &first_branch)?
            .into_iter()
            .filter_map(|pr| Some((branch_number(issue, &pr.head)?, pr)))
            .collect();

    let highest = on_origin
        .iter()
        .map(|(number, _)| *number)
        .chain(prs.iter().map(|(number, _)| *number))
        .max();
    let Some(highest) = highest else {
        check_local_branch(launch, &first_branch, None)?;
        return Ok(Selection::Fresh {
            branch: first_branch,
        });
    };
    let branch = branch_name(issue, highest);
    // A branch can have had several PRs; the newest one decides.
    let pr = prs
        .into_iter()
        .filter_map(|(number, pr)| (number == highest).then_some(pr))
        .max_by_key(|pr| pr.number);
    let origin_sha = on_origin
        .into_iter()
        .find_map(|(number, sha)| (number == highest).then_some(sha));
    match (origin_sha, pr) {
        (Some(origin_sha), pr) if pr.as_ref().is_none_or(|pr| pr.state == PrState::Open) => {
            check_local_branch(launch, &branch, Some(&origin_sha))?;
            Ok(Selection::Continuation { branch, pr })
        }
        (_, pr) => bail!(
            "{branch} can't be continued ({}), and numbered Issue branches are not supported yet",
            match pr {
                Some(pr) => format!("its PR {} is {}", pr.url, pr.state),
                None => "it is gone from origin".to_string(),
            }
        ),
    }
}

/// A local `branch` in the launch repository must be at `origin_sha`, or not
/// exist when origin has no copy, so no local-only commits are destroyed.
fn check_local_branch(launch: &Git, branch: &str, origin_sha: Option<&str>) -> Result<()> {
    let Ok(local_sha) = launch.run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ]) else {
        return Ok(());
    };
    match origin_sha {
        Some(origin_sha) if origin_sha == local_sha => Ok(()),
        Some(_) => bail!(
            "the local branch {branch} differs from origin/{branch}; push, reset or delete it first"
        ),
        None => {
            bail!("the local branch {branch} is not on origin; push, rename or delete it first")
        }
    }
}

/// `issue-<n>` is branch 1, `issue-<n>-branch-<k>` is branch k for k ≥ 2.
fn branch_number(issue: &IssueUrl, branch: &str) -> Option<u64> {
    let rest = branch.strip_prefix(&branch_name(issue, 1))?;
    if rest.is_empty() {
        return Some(1);
    }
    let k: u64 = rest.strip_prefix("-branch-")?.parse().ok()?;
    (k >= 2 && branch == branch_name(issue, k)).then_some(k)
}

fn branch_name(issue: &IssueUrl, number: u64) -> String {
    match number {
        1 => format!("issue-{}", issue.number),
        k => format!("issue-{}-branch-{k}", issue.number),
    }
}
