//! Author-pass conflict admission and durable notification suppression.

use std::fs;
use std::io::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

#[derive(Serialize, Deserialize)]
struct Suppression {
    repo: String,
    number: u64,
    branch: String,
    fingerprint: String,
}

/// The caller holds the PR's origin Reservation through inspection and recording.
pub(super) fn eligible(
    launch: &Git,
    github: &GitHub,
    pr: &PullRequestUrl,
    flags: &Flags,
    config: &UserConfig,
) -> Result<bool> {
    let snapshot = github.review_pull_request(pr)?;
    launch.fetch(&[&snapshot.branch, &snapshot.head])?;
    let key = format!(
        "{}:{}:{}",
        pr.issue.repo().slug().to_lowercase(),
        pr.issue.number,
        snapshot.branch
    );
    let folder = crate::config::home()?.join(".thirdshift/review-conflicts");
    let path = folder.join(format!("{:x}.json", Sha256::digest(key.as_bytes())));
    let conflict = SkillWorktree::pull_request_conflict(launch, &snapshot.branch, &snapshot.head)?;
    let Some(conflict) = conflict else {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot clear Review conflict suppression"),
        }
        return Ok(true);
    };
    let previous = match fs::read(&path) {
        Ok(bytes) => Some(serde_json::from_slice::<Suppression>(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("cannot read Review conflict suppression"),
    };
    let unchanged = previous.is_some_and(|record| record.fingerprint == conflict.fingerprint);
    progress::step(format_args!(
        "skipping {}: local branch/worktree conflict{}",
        pr.url,
        if unchanged {
            " (unchanged; notification suppressed)"
        } else {
            ""
        }
    ));
    if unchanged {
        return Ok(false);
    }
    let mut notification =
        RunNotification::new(flags.notification(config), &config.email, About::ReviewRun)?;
    notification.pull_request(pr, &snapshot.title);
    fs::create_dir_all(&folder)?;
    let mut temporary = tempfile::NamedTempFile::new_in(&folder)?;
    serde_json::to_writer(
        &mut temporary,
        &Suppression {
            repo: pr.issue.repo().slug(),
            number: pr.issue.number,
            branch: snapshot.branch,
            fingerprint: conflict.fingerprint,
        },
    )?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path)?;
    progress::step(format_args!("{}: {:#}", pr.url, conflict.error));
    notification.finish(&Account {
        recovery: None,
        outcome: "failed",
        ended: Err(Cause::of(&conflict.error)),
        interrupted: false,
        pr_url: Some(&pr.url),
        advice: &[],
        base_fix: None,
        log: None,
        ticket_lines: &[],
        review: None,
        triage: None,
        failed_reproduction: None,
        pre_audit_failure: None,
        security_findings: None,
        security_findings_waiting: false,
        security_fix: None,
        security_close: None,
        urls: vec![&pr.url],
    });
    Ok(false)
}
