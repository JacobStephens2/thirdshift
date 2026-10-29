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
            launch: &directory,
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
    if fixing {
        match records.next_fix() {
            Ok(Some((record, metadata))) => {
                return fix(outside, repo, base, record, metadata, None, closing);
            }
            Ok(None) => {}
            Err(error) => return Outcome::Audited(error.into()),
        }
    }
    match outside.base_unchanged_since_security_audit() {
        Ok(true) => {
            let skipped = Skipped::UnchangedBase(base.to_string());
            outside.skipped(Pass::Security, &skipped);
            return Outcome::Skipped(skipped);
        }
        Ok(false) => {}
        Err(error) => return Outcome::Audited(error.into()),
    }
    let audited = audit_and_record(outside, repo, base, offer_command);
    if fixing && audited.outcome.is_ok() {
        match outside
            .security_records()
            .and_then(|records| records.next_fix())
        {
            Ok(Some((record, metadata))) => {
                return fix(
                    outside,
                    repo,
                    base,
                    record,
                    metadata,
                    Some(Box::new(audited)),
                    closing,
                );
            }
            Ok(None) => {}
            Err(error) => {
                return Outcome::Audited(Ended {
                    outcome: Err(error.into()),
                    ..audited
                });
            }
        }
    }
    Outcome::Audited(audited)
}

/// Dispatch the fix for `record`, after `audit` if this Security run audited.
fn fix(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    record: crate::github::SecurityRecord,
    metadata: RecordedFinding,
    audit: Option<Box<Ended>>,
    closing: bool,
) -> Outcome {
    let mut log = None;
    let outcome = (|| -> Result<(crate::run::Ended, crate::issue::IssueUrl)> {
        outside.check_harness(true)?;
        outside.started(Work::SecurityRun(repo));
        let (published, session_log) = outside.publish_security_fix(base, &record, &metadata.url);
        log = session_log;
        let issue = published?;
        outside.link_security_fix(&record, &issue)?;
        let mut ended = outside.dispatch(crate::pass::Dispatch::SecurityFix {
            issue: &issue,
            is_spec: record.fix_size()? == reproduction::FixSize::Spec,
            base,
            finding: &metadata,
        });
        if let Err(error) =
            outside.record_security_fix_ending(&record, &issue, ended.outcome.is_ok())
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
    let (ended, issue) = match outcome {
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
    };
    let close = match (&ended.outcome, &issue) {
        (Ok(reached), Some(issue))
            if closing && reached.resolution.goal() == Some(crate::run::Goal::Merged) =>
        {
            close_after_merge(outside, base, &record, issue, reached.resolution.url())
        }
        _ => CloseOutcome::NotAttempted,
    };
    Outcome::Fixed {
        ended,
        finding: metadata,
        audit,
        close,
    }
}

/// Close the finding whose fix merged as `pr_url`, after re-running its
/// proof-of-concept test against the merged Base branch. A check that does
/// not pass, or any step that fails, leaves the record open with a private
/// note of what happened, and never changes the fix's outcome.
fn close_after_merge(
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

fn audit_and_record(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    offer_command: Option<&str>,
) -> Ended {
    let mut advice = Vec::new();
    let mut findings = Vec::new();
    let mut waiting = Vec::new();
    let mut log = None;
    let mut reproduction = None;
    let mut audit_started = false;
    let recorded = (|| -> Result<Recorded> {
        outside.check_harness(false)?;
        outside.check_node()?;
        outside.started(Work::SecurityRun(repo));
        let attempt = outside.audit(base);
        audit_started = attempt.started;
        log = attempt.log;
        let audited = attempt.outcome?;
        let mut recorded = Recorded {
            created: 0,
            existing: 0,
        };
        if audited.findings.is_empty() {
            return Ok(recorded);
        }
        let mut known = outside.security_records()?;
        let mut records = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for finding in audited.findings {
            let resolved = known.record_or_reuse(&finding, |records, draft| {
                outside.create_security_record(records, draft)
            })?;
            if resolved.created {
                recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
            } else {
                recorded.existing += 1;
            }
            let record = resolved.record;
            if record.untriaged() {
                waiting.push(record.metadata().url.clone());
            }
            let mut metadata = record.metadata().clone();
            let url = metadata.url.clone();
            let first = seen.insert(finding.fingerprint);
            // A new record's text is the audit's write-up, which may quote an
            // outcome no reproduction wrote.
            let kept = (!resolved.created)
                .then(|| record.completed_reproduction())
                .flatten();
            match (record.untriaged(), kept) {
                (false, _) => outside.step(format!(
                    "keeping the Day shift's grade for {}",
                    record.name()
                )),
                // A completed reproduction is not repeated while the record waits.
                (true, Some(outcome)) => {
                    metadata.severity = listed_severity(&outcome, offer_command, &mut advice);
                    if first {
                        outside.step(format!(
                            "keeping the completed Security reproduction of {}: {outcome}",
                            record.name()
                        ));
                    }
                }
                (true, None) if first => records.push((record, url)),
                (true, None) => {}
            }
            findings.push(metadata);
        }
        for (index, (record, url)) in records.iter().enumerate() {
            let number = index + 1;
            outside.step(format!(
                "starting Security reproduction {number} of {}",
                record.name()
            ));
            reproduction = Some(number);
            let (reproduced, session_log) = outside.reproduce(record, number);
            if session_log.is_some() {
                log = session_log;
            }
            let reproduced = reproduced?;
            outside.update_security_record(record, &reproduced)?;
            let severity = listed_severity(&reproduced.outcome, offer_command, &mut advice);
            for finding in findings.iter_mut().filter(|finding| finding.url == *url) {
                finding.severity = severity.clone();
            }
            outside.step(format!(
                "Security reproduction {number}: {}",
                reproduced.outcome
            ));
        }
        Ok(recorded)
    })();
    let failed_reproduction = if recorded.is_err() {
        reproduction
    } else {
        None
    };
    Ended {
        outcome: recorded.map_err(|error| FailedRun {
            log,
            ..error.into()
        }),
        audit_started,
        failed_reproduction,
        findings,
        waiting,
        advice,
    }
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
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, widgets};

    #[test]
    fn a_reproduced_fix_precedes_the_unchanged_base_but_yields_to_a_ready_issue() {
        for ready in [false, true] {
            let mut outside = InMemory::default().advisories(vec![
                serde_json::json!({"state":"draft", "severity":null}),
                serde_json::json!({
                    "state":"draft", "severity":"critical", "ghsa_id":"GHSA-first",
                    "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
                    "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
                }),
                serde_json::json!({
                    "state":"draft", "severity":"critical", "ghsa_id":"GHSA-second",
                    "summary":"Other bound", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-second",
                    "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
                }),
            ]).unchanged_base().dispatched_ending(crate::pass::ready_for_review());
            if ready {
                outside = outside.ready(7, false);
            }
            let outcome = run_through(&mut outside, &widgets(), "main", true, false, None);
            if ready {
                assert!(matches!(outcome, Outcome::Skipped(Skipped::ReadyIssue(_))));
                assert!(matches!(
                    &outside.calls[..],
                    [Call::ReadySearch, Call::Skipped(_)]
                ));
            } else {
                assert!(matches!(
                    outcome,
                    Outcome::Fixed {
                        ended: crate::run::Ended { outcome: Ok(_), .. },
                        ..
                    }
                ));
                assert!(
                    outside
                        .calls
                        .contains(&Call::PublishSecurityFix("GHSA-first".into()))
                );
                assert!(outside.calls.contains(&Call::DispatchSecurityFix {
                    issue: 8,
                    is_spec: false,
                    base: "main".into()
                }));
                assert!(!outside.calls.contains(&Call::AuditHistory));
                assert!(!outside.calls.contains(&Call::NodeCheck));
            }
        }
    }

    /// A dispatched fix that ended merged, for close-gate tests.
    fn merged(pr_url: &str) -> crate::run::Ended {
        crate::run::Ended {
            outcome: Ok(crate::run::Reached {
                resolution: crate::run::Resolution::PullRequest {
                    url: pr_url.to_string(),
                    goal: crate::run::Goal::Merged,
                },
                log: None,
                ticket_lines: Vec::new(),
            }),
            base_fix: None,
            advice: Vec::new(),
        }
    }

    const MERGED_PR: &str = "https://github.com/acme/widgets/pull/12";

    /// One reproduced advisory awaiting its fix, as the fix flow reads it.
    fn reproduced_advisory() -> serde_json::Value {
        serde_json::json!({
            "state":"draft", "severity":"critical", "ghsa_id":"GHSA-first",
            "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
            "description":"Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n"
        })
    }

    #[test]
    fn a_merged_fix_with_closing_allowed_closes_the_finding_naming_its_pull_request() {
        for private in [false, true] {
            let record = if private {
                serde_json::json!({
                    "number": 7, "state": "open", "title": "Bound input",
                    "html_url": "https://github.com/acme/widgets/issues/7",
                    "body": "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n",
                    "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
                })
            } else {
                reproduced_advisory()
            };
            let mut outside = if private {
                InMemory::default().finding_issues(vec![record])
            } else {
                InMemory::default().advisories(vec![record])
            }
            .dispatched_ending(merged(MERGED_PR))
            .close_check(true, "the bound holds; exit 0");
            let Outcome::Fixed { ended, close, .. } =
                run_through(&mut outside, &widgets(), "main", true, true, None)
            else {
                panic!("private {private}: fix expected");
            };
            assert!(ended.outcome.is_ok(), "private {private}");
            assert_eq!(
                close,
                CloseOutcome::Closed {
                    pr_url: MERGED_PR.into()
                },
                "private {private}"
            );
            let record_name = if private { "issue #7" } else { "GHSA-first" };
            let at = |call: &Call| outside.calls.iter().position(|c| c == call);
            let dispatched = at(&Call::DispatchSecurityFix {
                issue: 8,
                is_spec: false,
                base: "main".into(),
            })
            .expect("private {private}: no dispatch");
            let rechecked = at(&Call::RecheckSecurityFix {
                record: record_name.into(),
                base: "main".into(),
            })
            .expect("private {private}: no recheck");
            let closed = at(&Call::CloseSecurityRecord {
                issue: 8,
                pr: MERGED_PR.into(),
            })
            .expect("private {private}: no close");
            assert!(
                dispatched < rechecked && rechecked < closed,
                "private {private}: out of order: {:?}",
                outside.calls
            );
            assert!(
                !outside
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { .. })),
                "private {private}: a close must leave no note"
            );
            assert!(
                outside.calls.iter().any(|call| matches!(
                    call,
                    Call::Step(line) if line.contains(&format!("closed {record_name} by {MERGED_PR}"))
                )),
                "private {private}: no closed step: {:?}",
                outside.calls
            );
        }
    }

    #[test]
    fn without_closing_allowed_or_without_a_merge_a_fix_closes_nothing() {
        // Closing off, or the fix left ready for review, attempts no close.
        for (closing, ending, expected) in [
            (false, merged(MERGED_PR), CloseOutcome::NotAttempted),
            (
                true,
                crate::pass::ready_for_review(),
                CloseOutcome::NotAttempted,
            ),
        ] {
            let mut outside = InMemory::default()
                .advisories(vec![reproduced_advisory()])
                .dispatched_ending(ending);
            let Outcome::Fixed { ended, close, .. } =
                run_through(&mut outside, &widgets(), "main", true, closing, None)
            else {
                panic!("fix expected");
            };
            assert!(ended.outcome.is_ok());
            assert_eq!(close, expected);
            assert!(
                !outside.calls.iter().any(|call| matches!(
                    call,
                    Call::RecheckSecurityFix { .. }
                        | Call::CloseSecurityRecord { .. }
                        | Call::NoteSecurityCloseLeftOpen { .. }
                )),
                "closing {closing}: a close was attempted: {:?}",
                outside.calls
            );
        }
    }

    #[test]
    fn a_failing_proof_of_concept_leaves_the_finding_open_with_a_note() {
        let mut outside = InMemory::default()
            .advisories(vec![reproduced_advisory()])
            .dispatched_ending(merged(MERGED_PR))
            .close_check(false, "the bound still overflows; exit 1");
        let Outcome::Fixed { ended, close, .. } =
            run_through(&mut outside, &widgets(), "main", true, true, None)
        else {
            panic!("fix expected");
        };
        // The run's outcome stays the fix's outcome.
        assert!(ended.outcome.is_ok());
        let CloseOutcome::LeftOpen { reason } = close else {
            panic!("left open expected");
        };
        assert!(reason.contains("did not pass"), "{reason}");
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::CloseSecurityRecord { .. })),
            "a failed check must close nothing"
        );
        assert!(
            outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { issue: 8, .. })),
            "no left-open note: {:?}",
            outside.calls
        );
        assert!(
            outside.calls.iter().any(|call| matches!(
                call,
                Call::Step(line) if line.contains("left GHSA-first open")
            )),
            "no left-open step: {:?}",
            outside.calls
        );
    }

    #[test]
    fn a_failed_recheck_or_close_leaves_the_finding_open_without_changing_the_outcome() {
        for (recheck, failing) in [
            (
                InMemory::default().close_check_error("the sandbox is unavailable"),
                false,
            ),
            (InMemory::default().close_check(true, "exit 0"), true),
        ] {
            let outside = recheck
                .advisories(vec![reproduced_advisory()])
                .dispatched_ending(merged(MERGED_PR));
            let mut outside = if failing {
                outside.close_failing()
            } else {
                outside
            };
            let Outcome::Fixed { ended, close, .. } =
                run_through(&mut outside, &widgets(), "main", true, true, None)
            else {
                panic!("fix expected");
            };
            assert!(ended.outcome.is_ok());
            let CloseOutcome::LeftOpen { reason } = close else {
                panic!("left open expected");
            };
            if failing {
                assert!(reason.contains("could not close"), "{reason}");
                assert!(outside.calls.contains(&Call::CloseSecurityRecord {
                    issue: 8,
                    pr: MERGED_PR.into()
                }));
            } else {
                assert!(reason.contains("could not re-run"), "{reason}");
            }
            assert!(
                outside
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::NoteSecurityCloseLeftOpen { issue: 8, .. })),
                "no left-open note: {:?}",
                outside.calls
            );
        }
    }

    #[test]
    fn a_failed_left_open_note_still_leaves_the_outcome_unchanged() {
        let mut outside = InMemory::default()
            .advisories(vec![reproduced_advisory()])
            .dispatched_ending(merged(MERGED_PR))
            .close_check(false, "the bound still overflows; exit 1")
            .note_failing();
        let Outcome::Fixed { ended, close, .. } =
            run_through(&mut outside, &widgets(), "main", true, true, None)
        else {
            panic!("fix expected");
        };
        assert!(ended.outcome.is_ok());
        assert!(matches!(close, CloseOutcome::LeftOpen { .. }));
        assert!(
            outside.calls.iter().any(|call| matches!(
                call,
                Call::Step(line) if line.contains("could not note why GHSA-first stays open")
            )),
            "no note-failure step: {:?}",
            outside.calls
        );
    }

    #[test]
    fn a_closed_finding_is_neither_waiting_nor_fixed_again() {
        let closed = "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: succeeded\n\n<!-- thirdshift:security-close -->\nClosed by: https://github.com/acme/widgets/pull/12\n";
        let mut outside = InMemory::default().advisories(vec![serde_json::json!({
            "state":"closed", "severity":"critical", "ghsa_id":"GHSA-first",
            "summary":"Bound input", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-first",
            "description": closed,
        })]);
        let outcome = run_through(&mut outside, &widgets(), "main", true, true, None);
        assert!(
            matches!(outcome, Outcome::Audited(Ended { outcome: Ok(_), .. })),
            "a closed finding must not dispatch another fix"
        );
        assert!(
            !outside.calls.iter().any(|call| matches!(
                call,
                Call::DispatchSecurityFix { .. }
                    | Call::RecheckSecurityFix { .. }
                    | Call::CloseSecurityRecord { .. }
            )),
            "a closed finding was picked up: {:?}",
            outside.calls
        );
    }

    #[test]
    fn a_finding_waiting_for_the_day_shift_does_not_hold_off_the_audit() {
        let reproduced = "Private record\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced high single\n";
        for fixing in [false, true] {
            for private in [false, true] {
                // With fixing allowed, a reproduced finding is fixed instead.
                for description in ["Private record\n", reproduced] {
                    if fixing && description == reproduced {
                        continue;
                    }
                    let mut outside = if private {
                        InMemory::default().finding_issues(vec![serde_json::json!({
                            "state": "open", "body": description,
                            "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
                        })])
                    } else {
                        InMemory::default().advisories(vec![serde_json::json!({
                            "state": "draft", "description": description,
                            "severity": if description == reproduced { Some("high") } else { None }
                        })])
                    };
                    let outcome =
                        run_through(&mut outside, &widgets(), "main", fixing, false, None);
                    assert!(
                        matches!(outcome, Outcome::Audited(Ended { outcome: Ok(_), .. })),
                        "fixing {fixing}, private {private}: {description}"
                    );
                    assert_eq!(
                        outside.calls[..7],
                        [
                            Call::ReadySearch,
                            Call::AdvisoryList,
                            Call::AuditHistory,
                            Call::HarnessCheck,
                            Call::NodeCheck,
                            Call::Started(None),
                            Call::SecurityAudit("main".to_string())
                        ]
                    );
                }
            }
        }
    }

    #[test]
    fn checks_prerequisites_then_audits_without_pulling() {
        let mut outside = InMemory::default();
        let outcome = run_through(&mut outside, &widgets(), "main", false, false, None);
        assert!(matches!(
            outcome,
            Outcome::Audited(Ended { outcome: Ok(_), .. })
        ));
        assert_eq!(
            outside.calls,
            vec![
                Call::ReadySearch,
                Call::AdvisoryList,
                Call::AuditHistory,
                Call::HarnessCheck,
                Call::NodeCheck,
                Call::Started(None),
                Call::SecurityAudit("main".to_string())
            ]
        );
    }
    #[test]
    fn a_ready_issue_precedes_every_prerequisite_and_audit() {
        let mut outside = InMemory::default()
            .ready(7, false)
            .advisories(vec![serde_json::json!({"state":"draft", "severity":null})])
            .unchanged_base()
            .node_failing()
            .harness_failing();
        assert!(matches!(
            run_through(&mut outside, &widgets(), "main", false, false, None),
            Outcome::Skipped(Skipped::ReadyIssue(_))
        ));
        assert!(matches!(
            &outside.calls[..],
            [Call::ReadySearch, Call::Skipped(_)]
        ));
    }

    #[test]
    fn security_gates_choose_ready_then_failed_fix_then_unchanged_before_work() {
        let failed = "Private record\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: failed\n";
        for ready in [false, true] {
            for failed_fix in [false, true] {
                for unchanged in [false, true] {
                    // A finding waiting for the Day shift is no gate.
                    let mut advisories =
                        vec![serde_json::json!({"state":"draft", "severity":null})];
                    if failed_fix {
                        advisories.push(serde_json::json!({
                            "state": "draft", "severity": null, "description": failed,
                        }));
                    }
                    let mut outside = InMemory::default().advisories(advisories);
                    if ready {
                        outside = outside.ready(7, false);
                    }
                    if unchanged {
                        outside = outside.unchanged_base();
                    }
                    let outcome = run_through(&mut outside, &widgets(), "main", false, false, None);
                    let expected = if ready {
                        "Ready issue #7"
                    } else if failed_fix {
                        "failed Security fix #8 is still open"
                    } else if unchanged {
                        "Base branch main hasn't changed since the last completed Security audit"
                    } else {
                        assert!(matches!(
                            outcome,
                            Outcome::Audited(Ended { outcome: Ok(_), .. })
                        ));
                        assert!(outside.calls.contains(&Call::SecurityAudit("main".into())));
                        continue;
                    };
                    let Outcome::Skipped(skipped) = outcome else {
                        panic!("Security run should skip");
                    };
                    assert!(skipped.to_string().starts_with(expected), "{skipped}");
                    assert!(!outside.calls.contains(&Call::HarnessCheck));
                    assert_eq!(outside.calls.contains(&Call::AdvisoryList), !ready);
                    assert_eq!(
                        outside.calls.contains(&Call::AuditHistory),
                        !ready && !failed_fix
                    );
                }
            }
        }
    }

    #[test]
    fn a_failed_fix_yields_to_ready_work_but_precedes_audit_history() {
        for private in [false, true] {
            for ready in [false, true] {
                for closed in [false, true] {
                    for fixing in [false, true] {
                        let body = "Private record\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/8\nFix Run: failed\n";
                        let values = vec![
                            serde_json::json!({
                                "state": if private { "open" } else { "draft" },
                                "severity": null, "description": body, "body": body,
                                "security_fix_closed": closed,
                            }),
                            serde_json::json!({
                                "state": if private { "open" } else { "draft" },
                                "severity": null, "labels": [{"name":"needs-triage"}],
                            }),
                        ];
                        let mut outside = if private {
                            InMemory::default().finding_issues(values)
                        } else {
                            InMemory::default().advisories(values)
                        }
                        .unchanged_base();
                        if ready {
                            outside = outside.ready(7, false);
                        }
                        let Outcome::Skipped(skipped) =
                            run_through(&mut outside, &widgets(), "main", fixing, false, None)
                        else {
                            panic!("Security run should skip");
                        };
                        let expected = if ready {
                            "Ready issue #7"
                        } else if !closed {
                            "failed Security fix #8 is still open"
                        } else {
                            "Base branch main hasn't changed"
                        };
                        assert!(skipped.to_string().starts_with(expected), "{skipped}");
                        assert!(!outside.calls.contains(&Call::HarnessCheck));
                        assert_eq!(
                            outside.calls.contains(&Call::AuditHistory),
                            !ready && closed
                        );
                        assert_eq!(outside.calls.contains(&Call::AdvisoryList), !ready);
                    }
                }
            }
        }
    }

    #[test]
    fn each_failed_prerequisite_prevents_later_operations() {
        for (mut outside, expected) in [
            (
                InMemory::default().harness_failing(),
                vec![
                    Call::ReadySearch,
                    Call::AdvisoryList,
                    Call::AuditHistory,
                    Call::HarnessCheck,
                ],
            ),
            (
                InMemory::default().node_failing(),
                vec![
                    Call::ReadySearch,
                    Call::AdvisoryList,
                    Call::AuditHistory,
                    Call::HarnessCheck,
                    Call::NodeCheck,
                ],
            ),
        ] {
            assert!(matches!(
                run_through(&mut outside, &widgets(), "main", false, false, None),
                Outcome::Audited(Ended {
                    outcome: Err(_),
                    ..
                })
            ));
            assert_eq!(outside.calls, expected);
        }
    }

    #[test]
    fn a_failed_audit_records_nothing_and_keeps_its_session_log() {
        let mut outside = InMemory::default()
            .audit_failing("invalid report")
            .session_log("audit.jsonl");
        let Outcome::Audited(Ended {
            outcome: Err(failed),
            ..
        }) = run_through(&mut outside, &widgets(), "main", false, false, None)
        else {
            panic!("audit should fail");
        };
        assert_eq!(failed.log, Some(std::path::PathBuf::from("audit.jsonl")));
        assert_eq!(
            outside
                .calls
                .iter()
                .filter(|call| **call == Call::AdvisoryList)
                .count(),
            1
        );
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::CreateAdvisory(_)))
        );
    }

    #[test]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the test picks out advisories and reproductions among the calls it asserts on"
    )]
    fn repeated_fingerprints_preserve_counts_metadata_order_and_reproduce_once() {
        use crate::github::{DraftAdvisory, Package};
        use serde_json::json;
        let draft = |fingerprint: &str| DraftAdvisory {
            fingerprint: fingerprint.into(),
            summary: "Candidate".into(),
            description: format!("Fingerprint: `{fingerprint}`"),
            package: Package {
                ecosystem: "other".into(),
                name: None,
            },
        };
        let mut outside = InMemory::default()
            .audited(vec![draft("old"), draft("new"), draft("new")])
            .advisories(vec![
                json!({"ghsa_id":"old", "state":"closed", "description":"Fingerprint: `old`", "summary":"Candidate", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-old"}),
            ]);
        let Outcome::Audited(Ended {
            outcome: Ok(recorded),
            findings,
            ..
        }) = run_through(&mut outside, &widgets(), "main", false, false, None)
        else {
            panic!("audit should succeed");
        };
        assert_eq!((recorded.created, recorded.existing), (1, 2));
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.url.as_str())
                .collect::<Vec<_>>(),
            vec![
                "https://github.com/acme/widgets/security/advisories/GHSA-old",
                "https://github.com/acme/widgets/security/advisories/new",
                "https://github.com/acme/widgets/security/advisories/new",
            ]
        );
        assert_eq!(
            outside
                .calls
                .iter()
                .filter_map(|call| match call {
                    Call::CreateAdvisory(fingerprint) => Some(fingerprint.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["new"]
        );
        assert_eq!(
            outside
                .calls
                .iter()
                .filter_map(|call| match call {
                    Call::Reproduce(record) => Some(record.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["new"]
        );
    }

    #[test]
    fn a_refound_finding_keeps_its_completed_reproduction() {
        use crate::github::{DraftAdvisory, Package};
        use serde_json::json;
        let draft = |fingerprint: &str| DraftAdvisory {
            fingerprint: fingerprint.into(),
            summary: "Candidate".into(),
            description: format!("Fingerprint: `{fingerprint}`"),
            package: Package {
                ecosystem: "other".into(),
                name: None,
            },
        };
        // A failed, refused or interrupted reproduction leaves no outcome.
        let findings = [
            ("confirmed", Some("reproduced high spec"), Some("high")),
            ("refuted", Some("not reproduced"), None),
            ("unfinished", None, None),
        ];
        for private in [false, true] {
            let records = findings
                .iter()
                .enumerate()
                .map(|(index, (fingerprint, outcome, severity))| {
                    let description = match outcome {
                        Some(outcome) => format!(
                            "Fingerprint: `{fingerprint}`\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: {outcome}\n"
                        ),
                        None => format!("Fingerprint: `{fingerprint}`\n"),
                    };
                    if private {
                        json!({
                            "number": index + 1, "state": "open", "body": description,
                            "title": "Candidate",
                            "html_url": format!("https://github.com/acme/widgets/issues/{}", index + 1),
                            "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
                        })
                    } else {
                        json!({
                            "ghsa_id": fingerprint, "state": "draft", "description": description,
                            "summary": "Candidate", "severity": severity,
                            "html_url": format!("https://github.com/acme/widgets/security/advisories/{fingerprint}")
                        })
                    }
                })
                .collect();
            let outside = InMemory::default().audited(vec![
                draft("confirmed"),
                draft("refuted"),
                draft("unfinished"),
                draft("refuted"),
            ]);
            let mut outside = if private {
                outside.finding_issues(records)
            } else {
                // A reproduced advisory carries its severity, so it reads as graded.
                outside.advisories(records)
            };
            let Outcome::Audited(Ended {
                outcome: Ok(recorded),
                findings,
                ..
            }) = run_through(&mut outside, &widgets(), "main", false, false, None)
            else {
                panic!("audit should succeed");
            };
            assert_eq!((recorded.created, recorded.existing), (0, 4));
            let unfinished = if private { "issue #3" } else { "unfinished" };
            assert_eq!(
                outside
                    .calls
                    .iter()
                    .filter(|call| matches!(
                        call,
                        Call::Reproduce(_) | Call::UpdateSecurityRecord(_)
                    ))
                    .collect::<Vec<_>>(),
                vec![
                    &Call::Reproduce(unfinished.into()),
                    &Call::UpdateSecurityRecord(unfinished.into())
                ],
                "private {private}"
            );
            let refuted = if private { "issue #2" } else { "refuted" };
            assert_eq!(
                outside
                    .calls
                    .iter()
                    .filter(|call| matches!(call, Call::Step(line) if line.contains(refuted)))
                    .collect::<Vec<_>>(),
                vec![&Call::Step(format!(
                    "keeping the completed Security reproduction of {refuted}: not reproduced"
                ))]
            );
            assert_eq!(
                findings
                    .iter()
                    .map(|finding| finding.severity.as_deref())
                    .collect::<Vec<_>>(),
                vec![Some("high"), None, None, None]
            );
        }
    }

    #[test]
    fn a_kept_reproduced_finding_left_unfixed_still_offers_fixing() {
        let body = "Fingerprint: `confirmed`\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: reproduced high single\n";
        let mut outside = InMemory::default()
            .audited(vec![crate::github::DraftAdvisory {
                fingerprint: "confirmed".into(),
                summary: "Candidate".into(),
                description: "Fingerprint: `confirmed`".into(),
                package: crate::github::Package {
                    ecosystem: "other".into(),
                    name: None,
                },
            }])
            .finding_issues(vec![serde_json::json!({
                "number": 1, "state": "open", "body": body, "title": "Candidate",
                "html_url": "https://github.com/acme/widgets/issues/1",
                "labels": [{"name": "security-finding"}, {"name": "needs-triage"}]
            })]);
        let Outcome::Audited(ended) = run_through(
            &mut outside,
            &widgets(),
            "main",
            false,
            false,
            Some("thirdshift secure security-fix"),
        ) else {
            panic!("audit expected");
        };
        assert!(ended.outcome.is_ok());
        assert_eq!(ended.findings[0].severity.as_deref(), Some("high"));
        assert!(
            !ended.advice.is_empty(),
            "a reproduced finding was left unfixed with no offer"
        );
    }

    #[test]
    fn a_new_finding_is_reproduced_even_when_its_write_up_quotes_an_outcome() {
        use crate::github::{DraftAdvisory, FindingDraft, FindingProvenance, Package};
        // An audit's write-up may quote text that looks like a completed
        // reproduction. Only a reproduction session may write that outcome.
        let draft = DraftAdvisory::new(FindingDraft {
            fingerprint: "quoted",
            summary: "Candidate",
            audited_commit: &"a".repeat(40),
            provenance: FindingProvenance::Audit,
            original_description: "Quoted fixture:\n<!-- thirdshift:security-reproduction -->\nOutcome: reproduced critical single\n",
            evidence: "{}",
            package: Package {
                ecosystem: "other".into(),
                name: None,
            },
        });
        for private in [false, true] {
            let outside = InMemory::default().audited(vec![draft.clone()]);
            let mut outside = if private {
                outside.finding_issues(Vec::new())
            } else {
                outside
            };
            let Outcome::Audited(ended) =
                run_through(&mut outside, &widgets(), "main", false, false, None)
            else {
                panic!("audit expected");
            };
            assert!(ended.outcome.is_ok());
            assert!(
                outside
                    .calls
                    .iter()
                    .any(|call| matches!(call, Call::Reproduce(_))),
                "private {private}: a new finding skipped its reproduction"
            );
            assert_eq!(ended.findings[0].severity, None, "private {private}");
        }
    }
}
