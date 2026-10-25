//! Re-run a fixed finding's proof-of-concept test against the merged Base
//! branch, in the reproduction sandbox, before its private record is closed.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::git::Git;
use crate::github::SecurityRecord;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::prompt;
use crate::session::{Kind, Logs, Purpose, Sessions};
use crate::worktree::ReviewWorktree;

/// Whether the proof-of-concept test now passes on the merged Base branch,
/// with what the check ran and found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub passes: bool,
    pub detail: String,
}

pub fn recheck(
    launch: &Git,
    repo: &Repo,
    base: &str,
    pr_url: &str,
    record: &SecurityRecord,
    harness: &Choice,
) -> (Result<Check>, Option<PathBuf>) {
    let worktree = match ReviewWorktree::create(launch, &repo.name, base) {
        Ok(worktree) => worktree,
        Err(error) => return (Err(error), None),
    };
    let logs = Logs::of_security_run(repo);
    Sessions::within(&logs, worktree.path(), harness, |sessions| {
        let message = sessions.run_to_final_message(
            Purpose::Security,
            Kind::SecurityCloseCheck,
            &prompt::security_close_check(base, pr_url, record.description()),
        )?;
        let message = message.as_deref().unwrap_or_default().trim_end();
        let (detail, line) = message.rsplit_once('\n').unwrap_or(("", message));
        if line == prompt::CLOSE_CHECK_PASSES_LINE {
            Ok(Check {
                passes: true,
                detail: detail.trim().to_string(),
            })
        } else if let Some(reason) = line.strip_prefix(prompt::CLOSE_CHECK_FAILS_PREFIX) {
            if reason.trim().is_empty() || reason.contains(['\r', '\n']) {
                bail!("the Security close check ended with an invalid reason")
            }
            Ok(Check {
                passes: false,
                detail: reason.trim().to_string(),
            })
        } else {
            bail!("the Security close check ended without the final line its prompt asks for")
        }
    })
}
