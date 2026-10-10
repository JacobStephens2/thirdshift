//! Publish and check a terse fix Ticket or Spec, without exposing the private write-up.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::git::Git;
use crate::github::{GitHub, SecurityRecord};
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::prompt;
use crate::security::reproduction::FixSize;
use crate::session::{Logs, Purpose, Sessions};
use crate::worktree::ReviewWorktree;

pub const SECURITY_FIX: Label =
    Label::new("security-fix", "A fix for a reproduced Security finding");

pub fn publish(
    launch: &Git,
    repo: &Repo,
    base: &str,
    record: &SecurityRecord,
    url: &str,
    harness: &Choice,
) -> (Result<IssueUrl>, Option<PathBuf>) {
    let size = match record.fix_size() {
        Ok(size) => size,
        Err(error) => return (Err(error), None),
    };
    if let Some(number) = record.private_issue_number()
        && size == FixSize::Single
    {
        let ready = (|| -> Result<IssueUrl> {
            let issue = IssueUrl::parse(url)?;
            if issue.number != number || !issue.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
                bail!("the private Security finding's issue does not match its record");
            }
            let github = GitHub::new();
            let viewed = github.issue(&issue)?;
            if !viewed.is_open
                || viewed
                    .labels
                    .swapped(&[NEEDS_TRIAGE], &[])
                    .unready()
                    .is_some()
                || !github.tickets(&issue)?.is_empty()
            {
                bail!("the private Security finding must be an open, ready single Ticket");
            }
            if crate::interrupt::requested() {
                bail!("interrupted");
            }
            Edit::of(
                &issue,
                viewed.labels,
                &[NEEDS_TRIAGE],
                &[READY_FOR_AGENT, SECURITY_FIX],
            )
            .apply(&github)?;
            Ok(issue)
        })();
        return (ready, None);
    }
    let worktree = match ReviewWorktree::create(launch, &repo.name, base) {
        Ok(worktree) => worktree,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    let started = Utc::now();
    let prompt = prompt::security_fix(base, url, record.description());
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        let message =
            sessions.run_to_final_message(Purpose::Security, "security-fix-publishing", &prompt)?;
        let github = GitHub::new();
        // A single public fix stages its proposed text for validation;
        // thirdshift creates the issue from it. Anything else keeps the
        // session-created issue path until staged specs land.
        let staged = (size == FixSize::Single && record.private_issue_number().is_none())
            .then(|| message.as_deref().and_then(parse_staged_proposal))
            .flatten();
        let issue = match staged {
            Some((title, body)) => {
                // Validate the staged text before any public issue exists;
                // a rejection blocks creation, so no public issue leaks
                // private write-up text.
                record.check_public_fix_text(&format!("{title}\n\n{body}"), url)?;
                github.create_issue_in(repo, &title, &body, &[NEEDS_TRIAGE])?
            }
            None => {
                let line = message
                    .as_deref()
                    .unwrap_or_default()
                    .lines()
                    .map(str::trim)
                    .rfind(|line| !line.is_empty())
                    .unwrap_or_default();
                let prefix = match size {
                    FixSize::Single => prompt::SECURITY_FIX_LINE,
                    FixSize::Spec => prompt::SECURITY_FIX_SPEC_LINE,
                };
                let issue = line.strip_prefix(prefix).and_then(|url| IssueUrl::parse(url).ok()).context("the Security fix publishing session ended without the final line its prompt asks for")?;
                if !issue.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
                    bail!("the Security fix Ticket is not in this repository");
                }
                issue
            }
        };
        let private = record.private_issue_number().is_some();
        if let Some(number) = record.private_issue_number()
            && issue.number != number
        {
            bail!("a private Security fix must reuse the finding's own issue");
        }
        let viewed = github.issue(&issue)?;
        if !viewed.is_open {
            bail!("the Security fix Ticket is closed");
        }
        if !private && viewed.created.timestamp() < started.timestamp() {
            bail!("the Security fix Ticket was created before this session");
        }
        if !private && !viewed.labels.has(NEEDS_TRIAGE)
            || viewed
                .labels
                .swapped(&[NEEDS_TRIAGE], &[])
                .unready()
                .is_some()
        {
            bail!(
                "the Security fix Ticket must be labelled needs-triage with no other Unready Ticket label"
            );
        }
        let tickets = github.tickets(&issue)?;
        if (size == FixSize::Spec) == tickets.is_empty() {
            bail!("the Security fix's sub-issues do not match the reproduced fix size");
        }
        if !private {
            check_issue_text(&github, &issue, record, url)?;
        }
        for child in &tickets {
            let issue = issue.sibling(child.number);
            let viewed = github.issue(&issue)?;
            if !viewed.is_open || viewed.created.timestamp() < started.timestamp() {
                bail!("a Security fix Spec's Ticket must be new and open");
            }
            if child.has_sub_issues
                || !viewed.labels.has(READY_FOR_AGENT)
                || viewed.labels.unready().is_some()
            {
                bail!("a Security fix Spec's Ticket must be ready-for-agent with no sub-issues");
            }
            check_issue_text(&github, &issue, record, url)?;
        }
        for child in tickets {
            let issue = issue.sibling(child.number);
            Edit::of(&issue, github.issue_labels(&issue)?, &[], &[SECURITY_FIX]).apply(&github)?;
        }
        if crate::interrupt::requested() {
            bail!("interrupted");
        }
        Edit::of(
            &issue,
            viewed.labels,
            &[NEEDS_TRIAGE],
            &[READY_FOR_AGENT, SECURITY_FIX],
        )
        .apply(&github)?;
        Ok(issue)
    })
}

/// The single public fix the publishing session staged for validation:
/// its proposed `(title, body)`, instead of a session-created public issue.
fn parse_staged_proposal(message: &str) -> Option<(String, String)> {
    let (_, staged) = message.rsplit_once(prompt::STAGED_PROPOSAL_MARKER)?;
    let staged = staged.trim_start_matches(['\n', '\r']);
    let (title, body) = staged.split_once('\n')?;
    let title = title.strip_prefix(prompt::STAGED_TITLE_MARKER)?.trim();
    let body = body
        .strip_prefix(prompt::STAGED_BODY_MARKER)
        .map(|body| body.trim().to_string())?;
    if title.is_empty() || body.is_empty() {
        return None;
    }
    Some((title.to_string(), body))
}

fn check_issue_text(
    github: &GitHub,
    issue: &IssueUrl,
    record: &SecurityRecord,
    url: &str,
) -> Result<()> {
    let body = github.security_fix_text(issue)?;
    record.check_public_fix_text(&body, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_proposal_parses_title_and_body() {
        let message = "Some reasoning\nProposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: https://example.invalid/advisory\n";
        let (title, body) = parse_staged_proposal(message).expect("staged proposal parses");
        assert_eq!(title, "Bound accepted input");
        assert!(body.contains("Reject oversized input."));
    }

    #[test]
    fn staged_proposal_rejects_a_bare_url_final_line() {
        let message = "Security fix Ticket: https://github.com/acme/widgets/issues/8\n";
        assert!(parse_staged_proposal(message).is_none());
    }
}
