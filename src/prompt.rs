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
         Include \"Closes #{number}\" in the PR body.\n",
        url = issue.url,
        number = issue.number,
    )
}
