//! Issue holds that keep an author Pass out of another Command's Delivery.

use super::*;

pub(super) fn eligible(launch: &Git, github: &GitHub, pr: &PullRequestUrl) -> Result<bool> {
    let snapshot = github.review_pull_request(pr)?;
    let mut issues: Vec<_> = github
        .closing_issues(pr)?
        .into_iter()
        .map(|issue| (issue, "closing issue"))
        .collect();
    // A successful empty link observation is normal for a non-default base.
    // Errors still propagate; missing observations must never authorize work.
    if issues.is_empty() {
        issues.extend(
            body_closing_issues(pr, &snapshot.body)
                .into_iter()
                .map(|issue| (issue, "closing issue")),
        );
    }
    if let Some(spec) = crate::branch::issue_of_branch(&pr.issue.repo(), &snapshot.base) {
        issues.insert(0, (spec, "base Spec"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (issue, kind) in issues {
        if !seen.insert(issue.identity()) {
            continue;
        }
        let held = if github.issue_labels(&issue)?.has(crate::claim::IN_PROGRESS) {
            Some("an in-progress Claim from a live Command")
        } else if crate::reservation::Reservation::is_held(launch, &issue)? {
            Some("an issue reservation on its origin (possibly retained for recovery)")
        } else {
            None
        };
        if let Some(held) = held {
            progress::step(format_args!(
                "skipping #{} ({}): {kind} #{} ({}) has {held}",
                pr.issue.number, pr.url, issue.number, issue.url
            ));
            return Ok(false);
        }
    }
    Ok(true)
}

/// Local closing references in a PR body, used only when GitHub has no links.
fn body_closing_issues(pr: &PullRequestUrl, body: &str) -> Vec<crate::issue::IssueUrl> {
    let words: Vec<_> = body.split_whitespace().collect();
    words
        .windows(2)
        .filter_map(|pair| {
            let keyword = pair[0].trim_end_matches(':');
            if ![
                "close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves",
                "resolved",
            ]
            .iter()
            .any(|candidate| keyword.eq_ignore_ascii_case(candidate))
            {
                return None;
            }
            let reference = pair[1]
                .strip_prefix('#')?
                .trim_end_matches([',', '.', ';', ')', ']', '}']);
            if !reference.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let number = reference.parse::<std::num::NonZeroU64>().ok()?;
            crate::issue::IssueUrl::parse(&format!(
                "https://github.com/{}/issues/{number}",
                pr.issue.repo_slug()
            ))
            .ok()
        })
        .collect()
}
