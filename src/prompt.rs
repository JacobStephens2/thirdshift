//! The prompts agent sessions are started with.

use crate::issue::IssueUrl;

/// The fresh prompt, for a run that starts a new Issue branch.
pub fn fresh(issue: &IssueUrl, base: &str, branch: &str) -> String {
    format!(
        "/thirdshift:implement {url}\n\
         The base branch is {base}. Review with /thirdshift:code-review using {base} as the fixed point.\n\
         Address the Standards and Spec findings you agree with.\n\
         Push branch {branch} and create a pull request against {base} using /thirdshift:pr, marked ready for review.\n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         Include \"Closes #{number}\" in the PR body.\n",
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
         Resolve the conflicts, finish the merge, and push {branch}. Do not rebase or force-push.\n",
        url = issue.url,
    )
}
