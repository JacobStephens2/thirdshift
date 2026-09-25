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
