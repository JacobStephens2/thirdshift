//! Publish and check a terse fix Ticket, without exposing the private write-up.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::git::Git;
use crate::github::{GitHub, SecurityRecord};
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::prompt;
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
        let ticket = line.strip_prefix(prompt::SECURITY_FIX_LINE).and_then(|url| IssueUrl::parse(url).ok()).context("the Security fix publishing session ended without the final line its prompt asks for")?;
        if !ticket.repo_slug().eq_ignore_ascii_case(&repo.slug()) {
            bail!("the Security fix Ticket is not in this repository");
        }
        let github = GitHub::new();
        let viewed = github.issue(&ticket)?;
        if !viewed.is_open {
            bail!("the Security fix Ticket is closed");
        }
        if viewed.created.timestamp() < started.timestamp() {
            bail!("the Security fix Ticket was created before this session");
        }
        if !viewed.labels.has(NEEDS_TRIAGE)
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
        if github.candidate(&ticket, NEEDS_TRIAGE)?.has_sub_issues() {
            bail!("the Security fix must be a single Ticket");
        }
        let body = github.security_fix_text(&ticket)?;
        if !body.contains(url) {
            bail!("the Security fix Ticket does not link the private record");
        }
        // Reject copied write-up lines and test text. The session is also
        // instructed to publish only what the fix changes, never a paraphrase.
        for line in record.description().lines().filter(|line| {
            let line = line.trim();
            line.len() >= 16
                && !line.starts_with("Fingerprint:")
                && !line.starts_with("Audited commit:")
                && !line.starts_with("Outcome:")
                && !line.starts_with("Severity:")
                && !line.starts_with("Fix size:")
                && !line.starts_with('#')
                && !line.starts_with("<!--")
        }) {
            if body.contains(line.trim()) {
                bail!("the Security fix Ticket includes private write-up text");
            }
        }
        if crate::interrupt::requested() {
            bail!("interrupted");
        }
        Edit::of(
            &ticket,
            viewed.labels,
            &[NEEDS_TRIAGE],
            &[READY_FOR_AGENT, SECURITY_FIX],
        )
        .apply(&github)?;
        Ok(ticket)
    })
}
