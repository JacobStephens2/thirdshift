//! Security runs: gate before work, audit origin's Base branch, then record
//! and reproduce the findings privately for the Day shift.

use std::fmt;

use anyhow::Result;

use crate::asks::Flags;
use crate::base_fix::Advice;
use crate::config::UserConfig;
use crate::failed_run::FailedRun;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{LaunchAndGitHub, Outside};

pub mod audit;
pub mod close;
pub(crate) mod findings;
pub(crate) mod fixing;
pub mod reproduction;
pub(crate) mod review;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixAsk {
    Allow,
    Forbid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAsk {
    Allow,
    Forbid,
}

pub enum Outcome {
    Skipped(Skipped),
    Audited(Ended),
    /// A fix was dispatched for one Security finding.
    Fixed {
        /// How the fix's Run ended.
        ended: crate::run::Ended,
        /// The Security finding the fix was dispatched for.
        finding: RecordedFinding,
        /// The audit this Security run completed before the fix, if it
        /// audited; its findings include those left waiting for the Day shift.
        audit: Option<Box<Ended>>,
        /// What became of the attempt to close the fixed finding.
        close: CloseOutcome,
    },
}

/// What became of the attempt to close the finding a merged fix fixed. A
/// failed close never changes the fix's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseOutcome {
    /// Closing was not attempted: not allowed, or the fix did not merge.
    NotAttempted,
    /// The finding was closed by the merged pull request.
    Closed { pr_url: String },
    /// The finding was left open, with why; the Day shift was told.
    LeftOpen { reason: String },
}

/// The Security run's outcome and the private records it reached, including before a failure.
pub struct Ended {
    pub outcome: Result<Recorded, FailedRun>,
    /// Whether an audit session was requested after preparation succeeded.
    pub audit_started: bool,
    /// A failed reproduction's number; its audit and private recording completed.
    pub failed_reproduction: Option<usize>,
    pub findings: Vec<RecordedFinding>,
    /// The private links of the findings whose records wait for the Day
    /// shift's triage.
    pub waiting: Vec<String>,
    pub advice: Vec<Advice>,
}

impl From<anyhow::Error> for Ended {
    fn from(error: anyhow::Error) -> Self {
        Self {
            outcome: Err(error.into()),
            audit_started: false,
            failed_reproduction: None,
            findings: Vec::new(),
            waiting: Vec::new(),
            advice: Vec::new(),
        }
    }
}

pub use crate::github::RecordedFinding;

pub enum Skipped {
    AlreadyRunning(AlreadyRunning),
    ReadyIssue(ListedIssue),
    FailedFix(crate::issue::IssueUrl),
    UnchangedBase(String),
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(running) => running.fmt(f),
            Self::ReadyIssue(listed) => write!(
                f,
                "Ready issue #{} \"{}\" goes first: {}",
                listed.issue.number, listed.title, listed.issue.url
            ),
            Self::FailedFix(ticket) => write!(
                f,
                "failed Security fix #{} is still open for the Day shift: {}",
                ticket.number, ticket.url
            ),
            Self::UnchangedBase(base) => write!(
                f,
                "Base branch {base} hasn't changed since the last completed Security audit"
            ),
        }
    }
}

#[derive(Default)]
pub struct Recorded {
    pub created: usize,
    pub existing: usize,
}

impl fmt::Display for Recorded {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Security audit recorded {} new finding(s), {} already recorded",
            self.created, self.existing
        )
    }
}

pub fn run(
    base: Option<&str>,
    flags: &Flags,
    config: &UserConfig,
    harness: &mut Choice,
) -> Outcome {
    let offer_command = (flags.security_fix.is_none() && config.security_fix.is_none())
        .then(|| crate::args::retry_with_security_fix(base, flags));
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base) {
        Ok(Start::Clear(launch)) => launch,
        Ok(Start::AlreadyRunning(running)) => {
            logs::skipped(Pass::Security, &running.0, &running);
            return Outcome::Skipped(Skipped::AlreadyRunning(running));
        }
        Err(error) => return Outcome::Audited(error.into()),
    };
    run_through(
        &mut LaunchAndGitHub {
            launch: directory.git(),
            base: &base,
            repo: &repo,
            harness,
            flags,
            config,
            security_review_harness: None,
        },
        &repo,
        base.name(),
        flags.security_fix_allowed(config),
        flags.security_close_allowed(config),
        offer_command.as_deref(),
    )
}

fn run_through(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    fixing: bool,
    closing: bool,
    offer_command: Option<&str>,
) -> Outcome {
    match outside.ready_issue(crate::ready::PickupOrder::LowestFirst) {
        Ok(Some(ready)) => {
            let skipped = Skipped::ReadyIssue(ready.listed);
            outside.skipped(Pass::Security, &skipped);
            return Outcome::Skipped(skipped);
        }
        Ok(None) => {}
        Err(error) => return Outcome::Audited(error.into()),
    }
    let records = match outside.security_records() {
        Ok(records) => records,
        Err(error) => return Outcome::Audited(error.into()),
    };
    match records.failed_fix() {
        Ok(Some(ticket)) => {
            let skipped = Skipped::FailedFix(ticket);
            outside.skipped(Pass::Security, &skipped);
            return Outcome::Skipped(skipped);
        }
        Ok(None) => {}
        Err(error) => return Outcome::Audited(error.into()),
    }
    let pending = if fixing {
        match records.next_fix() {
            Ok(pending) => pending,
            Err(error) => return Outcome::Audited(error.into()),
        }
    } else {
        None
    };
    if pending.is_none() {
        match outside.base_unchanged_since_security_audit() {
            Ok(true) => {
                let skipped = Skipped::UnchangedBase(base.to_string());
                outside.skipped(Pass::Security, &skipped);
                return Outcome::Skipped(skipped);
            }
            Ok(false) => {}
            Err(error) => return Outcome::Audited(error.into()),
        }
    }
    crate::processes::security::run(
        outside,
        crate::processes::security::SecurityRun {
            repo,
            base,
            fixing,
            closing,
            offer_command,
            pending,
        },
    )
}

/// Publish, link, dispatch and record a reproduced fix through the same
/// privacy rules for a Security run and a catalogue Step. The caller owns
/// child Process selection and keeps the record for a later close Step.
pub(crate) fn fix_record<O: Outside>(
    outside: &mut O,
    repo: &Repo,
    base: &str,
    record: &crate::github::SecurityRecord,
    metadata: &RecordedFinding,
    dispatch: impl FnOnce(&mut O, &crate::issue::IssueUrl, reproduction::FixSize) -> crate::run::Ended,
) -> (crate::run::Ended, Option<crate::issue::IssueUrl>) {
    let mut log = None;
    let outcome = (|| -> Result<(crate::run::Ended, crate::issue::IssueUrl)> {
        outside.check_harness(true)?;
        outside.started(Work::SecurityRun(repo));
        let (published, session_log) = outside.publish_security_fix(base, record, &metadata.url);
        log = session_log;
        let issue = published?;
        let size = record.fix_size()?;
        outside.link_security_fix(record, &issue)?;
        let mut ended = dispatch(outside, &issue, size);
        if let Err(error) =
            outside.record_security_fix_ending(record, &issue, ended.outcome.is_ok())
        {
            ended.outcome = Err(match ended.outcome {
                Err(mut failed) => {
                    let cause =
                        format!("could not record the failed Security fix's ending: {error:#}");
                    failed.error = std::mem::replace(&mut failed.error, error).context(cause);
                    failed
                }
                Ok(reached) => FailedRun {
                    log: reached.log,
                    pr_url: reached
                        .resolution
                        .goal()
                        .map(|_| reached.resolution.url().to_string()),
                    ticket_lines: reached.ticket_lines,
                    ..error
                        .context("could not record the successful Security fix's ending")
                        .into()
                },
            });
        }
        Ok((ended, issue))
    })();
    match outcome {
        Ok((ended, issue)) => (ended, Some(issue)),
        Err(error) => (
            crate::run::Ended {
                outcome: Err(FailedRun {
                    log,
                    ..error.into()
                }),
                base_fix: None,
                advice: Vec::new(),
            },
            None,
        ),
    }
}

/// Close the finding whose fix merged as `pr_url`, after re-running its
/// proof-of-concept test against the merged Base branch. A check that does
/// not pass, or any step that fails, leaves the record open with a private
/// note of what happened, and never changes the fix's outcome.
pub(crate) fn close_after_merge(
    outside: &mut impl Outside,
    base: &str,
    record: &crate::github::SecurityRecord,
    issue: &crate::issue::IssueUrl,
    pr_url: &str,
) -> CloseOutcome {
    let (checked, _log) = outside.recheck_security_fix(base, record, pr_url);
    match checked {
        Ok(check) if check.passes => match outside.close_security_record(record, issue, pr_url) {
            Ok(()) => {
                outside.step(format!("closed {} by {pr_url}", record.name()));
                CloseOutcome::Closed {
                    pr_url: pr_url.to_string(),
                }
            }
            Err(error) => leave_open(
                outside,
                record,
                issue,
                format!("could not close the private record: {error:#}"),
            ),
        },
        Ok(check) => leave_open(
            outside,
            record,
            issue,
            format!(
                "the proof-of-concept test did not pass against the merged Base branch: {}",
                check.detail
            ),
        ),
        Err(error) => leave_open(
            outside,
            record,
            issue,
            format!(
                "could not re-run the proof-of-concept test against the merged Base branch: {error:#}"
            ),
        ),
    }
}

/// Leave the finding open with a private note of `reason`, telling the Day
/// shift on stderr. The fix's outcome is unchanged.
fn leave_open(
    outside: &mut impl Outside,
    record: &crate::github::SecurityRecord,
    issue: &crate::issue::IssueUrl,
    reason: String,
) -> CloseOutcome {
    if let Err(error) = outside.note_security_close_left_open(record, issue, &reason) {
        outside.step(format!(
            "could not note why {} stays open: {error:#}",
            record.name()
        ));
    }
    outside.step(format!("left {} open: {reason}", record.name()));
    CloseOutcome::LeftOpen { reason }
}

/// The severity a finding is listed with. A reproduced finding left unfixed
/// offers fixing when neither the command nor the User config decided.
fn listed_severity(
    outcome: &reproduction::Outcome,
    offer_command: Option<&str>,
    advice: &mut Vec<Advice>,
) -> Option<String> {
    let severity = outcome.severity()?;
    if let Some(command) = offer_command {
        *advice = vec![
            Advice {
                label: "Allow fixing",
                value: command.to_string(),
            },
            Advice {
                label: "Or set",
                value: "fix = true under [security] in ~/.thirdshift/config.toml".to_string(),
            },
        ];
    }
    Some(severity.name().to_string())
}

#[cfg(test)]
mod tests;
