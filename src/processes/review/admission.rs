//! Closing-issue holds that keep an author Pass out of another Command's Delivery.

use super::*;

pub(super) fn eligible(launch: &Git, github: &GitHub, pr: &PullRequestUrl) -> Result<bool> {
    for issue in github.closing_issues(pr)? {
        let held = if github.issue_labels(&issue)?.has(crate::claim::IN_PROGRESS) {
            Some("an in-progress Claim from a live Command")
        } else if crate::reservation::Reservation::is_held(launch, &issue)? {
            Some("an issue reservation on its origin (possibly retained for recovery)")
        } else {
            None
        };
        if let Some(held) = held {
            progress::step(format_args!(
                "skipping #{} ({}): closing issue #{} ({}) has {held}",
                pr.issue.number, pr.url, issue.number, issue.url
            ));
            return Ok(false);
        }
    }
    Ok(true)
}
