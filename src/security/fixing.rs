//! Publish and check a terse fix Ticket or Spec, without exposing the private write-up.

use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;

use crate::git::Git;
use crate::github::{GitHub, SecurityRecord};
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::Label;
use crate::prompt;
use crate::security::reproduction::FixSize;
use crate::session::{Logs, Purpose, Sessions};
use crate::worktree::ReviewWorktree;

mod publication;

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
    if record.private_issue_number().is_some() && size == FixSize::Single {
        return (
            publication::publish(
                &mut GitHub::new(),
                repo,
                record,
                url,
                size,
                publication::Source::ExistingFinding,
            ),
            None,
        );
    }
    let worktree = match ReviewWorktree::create(launch, &repo.name, base) {
        Ok(worktree) => worktree,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    let started = Utc::now();
    let prompt = prompt::security_fix(base, url, record.description());
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        let purpose = if record.private_issue_number().is_none() {
            Purpose::SecurityPublishing
        } else {
            Purpose::Security
        };
        let message = sessions.run_to_final_message(purpose, "security-fix-publishing", &prompt)?;
        publication::publish(
            &mut GitHub::new(),
            repo,
            record,
            url,
            size,
            publication::Source::Session {
                message: message.as_deref(),
                started,
            },
        )
    })
}
