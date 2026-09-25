//! The prompts agent sessions are started with.

use crate::github::Check;
use crate::issue::IssueUrl;

/// Every prompt ends with this. Sessions run with `claude -p`, which exits
/// once the agent ends its turn, killing any background task still running.
const HEADLESS: &str = "You run headless: nobody is watching, and ending your turn ends the session. \
    Run tests and other long commands in the foreground, raising the Bash timeout if needed. \
    Never end your turn while a background task you depend on is still running: ending the turn kills it.\n";

/// The fresh prompt, for a run that starts a new Issue branch.
pub fn fresh(issue: &IssueUrl, base: &str, branch: &str) -> String {
    format!(
        "/thirdshift:implement {url}\n\
         The base branch is {base}. Review with /thirdshift:code-review using {base} as the fixed point.\n\
         Address the Standards and Spec findings you agree with.\n\
         Push branch {branch} and create a pull request against {base} using /thirdshift:pr, marked ready for review.\n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         Include \"Closes #{number}\" in the PR body.\n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
    )
}

/// The continuation prompt, for a run that picks up an existing Issue branch.
/// With `pr_url`, the agent updates that PR; otherwise it creates one.
pub fn continuation(issue: &IssueUrl, base: &str, branch: &str, pr_url: Option<&str>) -> String {
    let pr = match pr_url {
        Some(url) => format!(
            "Update PR {url} using /thirdshift:pr, rewriting its body to cover the whole branch, marked ready for review."
        ),
        None => format!(
            "Create a pull request against {base} using /thirdshift:pr, marked ready for review."
        ),
    };
    format!(
        "/thirdshift:implement {url}\n\
         \n\
         You are continuing work on branch {branch}, which already has commits (see git log {base}..HEAD). Build on them; don't start over.\n\
         \n\
         The base branch is {base}. Review with /thirdshift:code-review using {base} as the fixed point.\n\
         \n\
         Address the Standards and Spec findings you agree with.\n\
         \n\
         Push branch {branch}.\n\
         \n\
         {pr}\n\
         \n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         \n\
         Include \"Closes #{number}\" in the PR body.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
    )
}

/// The conflict Repair prompt, for a merge of the Base branch left in progress
/// with conflicts.
pub fn conflict_repair(issue: &IssueUrl, base: &str, branch: &str, pr_url: &str) -> String {
    format!(
        "/thirdshift:resolving-merge-conflicts\n\
         \n\
         A merge of origin/{base} into {branch} is in progress in this worktree and has conflicts.\n\
         {branch} implements {url}; its pull request is {pr_url}.\n\
         \n\
         Resolve the conflicts, finish the merge, and push {branch}. Do not rebase or force-push.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
    )
}

/// The CI-fix Repair prompt, for red CI on the PR's head commit, listing
/// each check in `failed`.
pub fn ci_fix_repair(
    issue: &IssueUrl,
    base: &str,
    branch: &str,
    pr_url: &str,
    failed: &[Check],
) -> String {
    let checks: String = failed
        .iter()
        .map(|check| match &check.url {
            Some(url) => format!("- {}: {url}\n", check.name),
            None => format!("- {}\n", check.name),
        })
        .collect();
    format!(
        "CI failed on pull request {pr_url} (branch {branch}, implementing {url}).\n\
         \n\
         Failed checks:\n\
         {checks}\
         \n\
         Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.\n\
         Run the affected checks locally, commit, and push {branch}.\n\
         \n\
         If a failure is not caused by this branch (it is flaky, or also fails on {base}), do not change code for it. Instead, add it to a \"CI notes\" section of the pull request body with a one-line explanation.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
    )
}

/// The resume prompt, for a session whose `killed` background work, by
/// description, was killed when it ended its turn.
pub fn resume(killed: &[&str]) -> String {
    format!(
        "Your background work ({killed}) was killed when your turn ended, because ending the turn ends the session.\n\
         \n\
         Re-run whatever you were waiting on in the foreground, then finish your job.\n\
         \n\
         {HEADLESS}",
        killed = killed.join("; "),
    )
}
