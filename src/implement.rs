//! The implementation Step shared by Ticket runs and Custom processes.
//! Branch selection uses the same fresh/Continuation facts; diagnosis and
//! its successful early ending belong here rather than to publication.

use anyhow::{Context, Result, ensure};

use crate::delivery::{Opening, OpeningSession};
use crate::issue::IssueUrl;
use crate::labels::Labels;
use crate::{prompt, worktree::Worktree};

pub fn prompt(issue: &IssueUrl, base: &str, selection: &crate::branch::Selection) -> String {
    let branch = selection.branch();
    match selection {
        crate::branch::Selection::Fresh { .. } => prompt::fresh(issue, base, branch),
        crate::branch::Selection::Continuation { pr, .. } => {
            prompt::continuation(issue, base, branch, pr.as_ref().map(|pr| pr.url.as_str()))
        }
    }
}

pub enum Completed {
    Implemented(Option<String>),
    NotReproduced(String),
}

/// Retain the implementation prompt and Base fix flow, diagnosing ordinary
/// bug issues first. Flaky-test Tickets use recorded evidence; Security fixes
/// and Base fixes keep their own flow.
pub fn prepare(
    issue: &IssueUrl,
    base: &str,
    worktree: &Worktree,
    mut opening: Opening,
    labels: Option<&Labels>,
) -> Result<Opening> {
    if labels.is_some_and(|labels| labels.has(crate::labels::FLAKY_TEST)) {
        opening.prompt.push_str("\nThis is a Flaky-test Ticket. The recorded CI failure is the evidence: the same commit passed on re-run. Implement a fix for the test's dependence on timing or scheduling, or the race it exposes. Do not require reproduction on demand or end with needs-info because the failure does not reproduce.\n");
    } else if let Some(labels) = labels
        && labels.has(crate::labels::Label::new("bug", "A reported bug"))
        && !labels.has(crate::security::fixing::SECURITY_FIX)
    {
        opening.session = OpeningSession::BugDiagnosis;
        let merge_base = worktree.ahead_merge_base(base)?;
        opening.prompt = prompt::diagnosing_bugs(issue, &opening.prompt, merge_base.as_deref());
    }
    Ok(opening)
}

/// Run the implement session through its caller's checked session lifecycle,
/// including Resume, then validate the diagnosis handoff before publication.
pub fn run(
    issue: &IssueUrl,
    opening: &Opening,
    session: impl FnOnce(&str) -> Result<Option<String>>,
) -> Result<Completed> {
    let final_message = session(&opening.prompt)?;
    if opening.session == OpeningSession::BugDiagnosis
        && let Some(comment_url) = not_reproduced(issue, final_message.as_deref())?
    {
        return Ok(Completed::NotReproduced(comment_url));
    }
    Ok(Completed::Implemented(final_message))
}

/// Confirm the reporter handoff from label facts read by the caller's adapter.
pub fn confirm_needs_info(labels: &Labels) -> Result<()> {
    ensure!(
        labels.has(crate::labels::NEEDS_INFO) && !labels.has(crate::labels::READY_FOR_AGENT),
        "bug not reproduced ending requires needs-info with ready-for-agent absent"
    );
    Ok(())
}

/// Missing or malformed completion lines fail instead of publishing a PR.
fn not_reproduced(issue: &IssueUrl, final_message: Option<&str>) -> Result<Option<String>> {
    let line = final_message
        .and_then(|message| message.trim_end().lines().last())
        .unwrap_or("");
    if line == prompt::BUG_REPRODUCED_LINE {
        return Ok(None);
    }
    let url = line
        .strip_prefix(prompt::BUG_NOT_REPRODUCED_LINE)
        .context("bug diagnosis ended without a reproduced or not reproduced completion line")?;
    ensure!(
        url.split_once("#issuecomment-")
            .is_some_and(|(comment_issue, id)| {
                IssueUrl::parse(comment_issue)
                    .is_ok_and(|comment_issue| comment_issue.identity() == issue.identity())
                    && !id.is_empty()
                    && id.bytes().all(|b| b.is_ascii_digit())
                    && id.parse::<u64>().is_ok_and(|n| n > 0)
            }),
        "bug not reproduced ending must link a comment on {}",
        issue.url
    );
    Ok(Some(url.to_string()))
}
