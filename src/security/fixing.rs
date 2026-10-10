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
        let private = record.private_issue_number().is_some();
        if let Some(number) = record.private_issue_number()
            && issue.number != number
        {
            bail!("a private Security fix must reuse the finding's own issue");
        }
        let github = GitHub::new();
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

fn check_issue_text(
    github: &GitHub,
    issue: &IssueUrl,
    record: &SecurityRecord,
    url: &str,
) -> Result<()> {
    let body = github.security_fix_text(issue)?;
    if !body.contains(url) {
        bail!("the Security fix Ticket does not link the private record");
    }
    if record.contains_private_text(&body) {
        bail!("the Security fix Ticket includes private write-up text");
    }
    Ok(())
}
