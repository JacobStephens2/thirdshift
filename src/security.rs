//! Security runs: gate before work, audit origin's Base branch, then record
//! and reproduce the findings privately for the Day shift.

use std::fmt;

use anyhow::{Context, Result};
use serde_json::Value;

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
pub(crate) mod fixing;
pub mod reproduction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixAsk {
    Allow,
    Forbid,
}

pub enum Outcome {
    Skipped(Skipped),
    Audited(Ended),
    Fixed {
        ended: crate::run::Ended,
        findings: Vec<RecordedFinding>,
    },
}

/// The audit's outcome and the private records it reached, including before a failure.
pub struct Ended {
    pub outcome: Result<Recorded, FailedRun>,
    pub findings: Vec<RecordedFinding>,
    pub advice: Vec<Advice>,
}

impl From<anyhow::Error> for Ended {
    fn from(error: anyhow::Error) -> Self {
        Self {
            outcome: Err(error.into()),
            findings: Vec::new(),
            advice: Vec::new(),
        }
    }
}

/// Preserve the typed command, quoting its words so the offered command can be run.
fn command_with_fixing() -> String {
    let words = std::env::args()
        .skip(1)
        .map(|word| {
            if word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_/.:@=".contains(c))
                && !word.is_empty()
            {
                word
            } else {
                format!("'{}'", word.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>();
    format!("thirdshift {} security-fix", words.join(" "))
}

/// Only the metadata allowed in a Run notification; no private write-up.
#[derive(Debug, PartialEq, Eq)]
pub struct RecordedFinding {
    pub severity: Option<String>,
    pub title: String,
    pub url: String,
}

impl RecordedFinding {
    pub(crate) fn of_record(record: &Value) -> Result<Self> {
        Ok(Self {
            severity: record["severity"].as_str().map(String::from),
            title: record["summary"]
                .as_str()
                .or_else(|| record["title"].as_str())
                .context("the Security finding's private record has no title")?
                .to_string(),
            url: record["html_url"]
                .as_str()
                .context("the Security finding's private record has no link")?
                .to_string(),
        })
    }
}

pub enum Skipped {
    AlreadyRunning(AlreadyRunning),
    ReadyIssue(ListedIssue),
    FindingWaiting,
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
            Self::FindingWaiting => f.write_str("a Security finding is waiting for the Day shift"),
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
    let offer_command =
        (flags.security_fix.is_none() && config.security_fix.is_none()).then(command_with_fixing);
    run_through(
        &mut LaunchAndGitHub {
            launch: &directory,
            base: &base,
            repo: &repo,
            harness,
            flags,
            config,
        },
        &repo,
        base.name(),
        flags.security_fix_allowed(config),
        offer_command.as_deref(),
    )
}

fn run_through(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    fixing: bool,
    offer_command: Option<&str>,
) -> Outcome {
    match outside.ready_issue() {
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
            Ok(Some((record, metadata))) => return fix(outside, repo, base, record, metadata),
            Ok(None) => {}
            Err(error) => return Outcome::Audited(error.into()),
        }
    }
    if records.waiting_for_day_shift(fixing) {
        let skipped = Skipped::FindingWaiting;
        outside.skipped(Pass::Security, &skipped);
        return Outcome::Skipped(skipped);
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
                let outcome = fix(outside, repo, base, record, metadata);
                if let Outcome::Fixed { ended, .. } = outcome {
                    return Outcome::Fixed {
                        ended,
                        findings: audited.findings,
                    };
                }
                return outcome;
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

fn fix(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    record: crate::github::SecurityRecord,
    metadata: RecordedFinding,
) -> Outcome {
    let mut log = None;
    let ended = (|| -> Result<crate::run::Ended> {
        outside.check_harness()?;
        outside.started(Work::SecurityRun(repo));
        let (published, session_log) = outside.publish_security_fix(base, &record, &metadata.url);
        log = session_log;
        let issue = published?;
        outside.link_security_fix(&record, &issue)?;
        let mut ended = outside.dispatch(crate::pass::Dispatch::SecurityFix {
            issue: &issue,
            is_spec: record.fix_size()? == reproduction::FixSize::Spec,
            base,
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
                    pr_url: Some(reached.pr_url),
                    ticket_lines: reached.ticket_lines,
                    ..error
                        .context("could not record the successful Security fix's ending")
                        .into()
                },
            });
        }
        Ok(ended)
    })();
    Outcome::Fixed {
        ended: ended.unwrap_or_else(|error| crate::run::Ended {
            outcome: Err(FailedRun {
                log,
                ..error.into()
            }),
            base_fix: None,
            advice: Vec::new(),
        }),
        findings: vec![metadata],
    }
}

fn audit_and_record(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    offer_command: Option<&str>,
) -> Ended {
    let mut advice = Vec::new();
    let mut findings = Vec::new();
    let mut log = None;
    let recorded = (|| -> Result<Recorded> {
        outside.check_harness()?;
        outside.check_node()?;
        outside.started(Work::SecurityRun(repo));
        let (audited, session_log) = outside.audit(base);
        log = session_log;
        let audited = audited?;
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
            let (record, metadata) = if let Some(value) = known.finding(&finding) {
                recorded.existing += 1;
                (known.record(value)?, RecordedFinding::of_record(value)?)
            } else {
                let value = outside.create_security_record(&known, &finding)?;
                let record = known.record(&value)?;
                let metadata = RecordedFinding::of_record(&value)?;
                known.remember(value);
                recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
                (record, metadata)
            };
            let url = metadata.url.clone();
            findings.push(metadata);
            if record.untriaged() && seen.insert(finding.fingerprint) {
                records.push((record, url));
            } else if !record.untriaged() {
                outside.step(format!(
                    "keeping the Day shift's grade for {}",
                    record.name()
                ));
            }
        }
        for (index, (record, url)) in records.iter().enumerate() {
            let number = index + 1;
            outside.step(format!(
                "starting Security reproduction {number} of {}",
                record.name()
            ));
            let (reproduced, session_log) = outside.reproduce(record, number);
            if session_log.is_some() {
                log = session_log;
            }
            let reproduced = reproduced?;
            outside.update_security_record(record, &reproduced)?;
            if reproduced.severity().is_some()
                && let Some(command) = offer_command
            {
                advice = vec![
                    Advice {
                        label: "Allow fixing",
                        value: command.to_string(),
                    },
                    Advice {
                        label: "Or set",
                        value: "fix = true under [security] in ~/.thirdshift/config.toml"
                            .to_string(),
                    },
                ];
            }
            for finding in findings.iter_mut().filter(|finding| finding.url == *url) {
                finding.severity = reproduced
                    .severity()
                    .map(|severity| severity.name().to_string());
            }
            outside.step(format!(
                "Security reproduction {number}: {}",
                reproduced.outcome
            ));
        }
        Ok(recorded)
    })();
    Ended {
        outcome: recorded.map_err(|error| FailedRun {
            log,
            ..error.into()
        }),
        findings,
        advice,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, widgets};

    #[test]
    fn a_reproduced_fix_precedes_waiting_and_unchanged_but_yields_to_a_ready_issue() {
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
            let outcome = run_through(&mut outside, &widgets(), "main", true, None);
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

    #[test]
    fn a_draft_without_severity_waits_before_prerequisites_or_audit() {
        let mut outside = InMemory::default()
            .advisories(vec![serde_json::json!({
                "state": "draft", "severity": null,
                "ghsa_id": "GHSA-test", "summary": "Unchecked input size"
            })])
            .harness_failing();
        let outcome = run_through(&mut outside, &widgets(), "main", false, None);
        assert!(matches!(outcome, Outcome::Skipped(_)));
        assert_eq!(
            outside.calls,
            vec![
                Call::ReadySearch,
                Call::AdvisoryList,
                Call::Skipped("a Security finding is waiting for the Day shift".into()),
            ]
        );
    }

    #[test]
    fn advisory_triage_ends_the_wait() {
        for (state, severity, waits) in [
            ("draft", None, true),
            ("draft", Some("low"), false),
            ("draft", Some("high"), false),
            ("published", None, false),
            ("closed", None, false),
            ("triage", None, false),
        ] {
            let mut outside = InMemory::default().advisories(vec![serde_json::json!({
                "state": state, "severity": severity
            })]);
            let outcome = run_through(&mut outside, &widgets(), "main", false, None);
            assert_eq!(
                matches!(outcome, Outcome::Skipped(_)),
                waits,
                "{state}: {severity:?}"
            );
            assert_eq!(
                outside.calls.contains(&Call::SecurityAudit("main".into())),
                !waits
            );
        }
    }

    #[test]
    fn only_open_private_findings_still_needing_triage_wait() {
        for (state, labels, waits) in [
            ("open", vec!["security-finding", "needs-triage"], true),
            ("OPEN", vec!["security-finding", "Needs-Triage"], true),
            ("closed", vec!["security-finding", "needs-triage"], false),
            ("open", vec!["security-finding"], false),
            ("open", vec!["security-finding", "wontfix"], false),
        ] {
            let mut outside = InMemory::default().finding_issues(vec![serde_json::json!({
                "state": state,
                "labels": labels.iter().map(|name| serde_json::json!({"name":name})).collect::<Vec<_>>()
            })]);
            let outcome = run_through(&mut outside, &widgets(), "main", false, None);
            assert_eq!(
                matches!(outcome, Outcome::Skipped(_)),
                waits,
                "{state}: {labels:?}"
            );
            assert_eq!(
                outside.calls.contains(&Call::SecurityAudit("main".into())),
                !waits
            );
        }
    }

    #[test]
    fn checks_prerequisites_then_audits_without_pulling() {
        let mut outside = InMemory::default();
        let outcome = run_through(&mut outside, &widgets(), "main", false, None);
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
            run_through(&mut outside, &widgets(), "main", false, None),
            Outcome::Skipped(Skipped::ReadyIssue(_))
        ));
        assert!(matches!(
            &outside.calls[..],
            [Call::ReadySearch, Call::Skipped(_)]
        ));
    }

    #[test]
    fn security_gates_choose_ready_then_waiting_then_unchanged_before_work() {
        for ready in [false, true] {
            for waiting in [false, true] {
                for unchanged in [false, true] {
                    let mut outside = InMemory::default();
                    if ready {
                        outside = outside.ready(7, false);
                    }
                    if waiting {
                        outside = outside.advisories(vec![
                            serde_json::json!({"state":"draft", "severity":null}),
                        ]);
                    }
                    if unchanged {
                        outside = outside.unchanged_base();
                    }
                    let outcome = run_through(&mut outside, &widgets(), "main", false, None);
                    let expected = if ready {
                        "Ready issue #7"
                    } else if waiting {
                        "a Security finding is waiting for the Day shift"
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
                    assert!(skipped.to_string().starts_with(expected));
                    assert!(!outside.calls.contains(&Call::HarnessCheck));
                    assert_eq!(outside.calls.contains(&Call::AdvisoryList), !ready);
                    assert_eq!(
                        outside.calls.contains(&Call::AuditHistory),
                        !ready && !waiting
                    );
                }
            }
        }
    }

    #[test]
    fn a_failed_fix_yields_to_ready_work_but_precedes_waiting_and_audit_history() {
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
                            run_through(&mut outside, &widgets(), "main", fixing, None)
                        else {
                            panic!("Security run should skip");
                        };
                        let expected = if ready {
                            "Ready issue #7"
                        } else if !closed {
                            "failed Security fix #8 is still open"
                        } else {
                            "a Security finding is waiting for the Day shift"
                        };
                        assert!(skipped.to_string().starts_with(expected), "{skipped}");
                        assert!(!outside.calls.contains(&Call::HarnessCheck));
                        assert!(!outside.calls.contains(&Call::AuditHistory));
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
                run_through(&mut outside, &widgets(), "main", false, None),
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
        }) = run_through(&mut outside, &widgets(), "main", false, None)
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
    fn only_new_fingerprints_get_a_private_record() {
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
            .audited(vec![draft("old"), draft("new")])
            .advisories(vec![
                json!({"ghsa_id":"old", "state":"closed", "description":"Fingerprint: `old`", "summary":"Candidate", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-old"}),
            ]);
        let Outcome::Audited(Ended {
            outcome: Ok(recorded),
            ..
        }) = run_through(&mut outside, &widgets(), "main", false, None)
        else {
            panic!("audit should succeed");
        };
        assert_eq!((recorded.created, recorded.existing), (1, 1));
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
                .filter(|call| matches!(
                    call,
                    Call::CreateAdvisory(_) | Call::Reproduce(_) | Call::UpdateSecurityRecord(_)
                ))
                .collect::<Vec<_>>(),
            vec![
                &Call::CreateAdvisory("new".into()),
                &Call::Reproduce("new".into()),
                &Call::UpdateSecurityRecord("new".into()),
            ]
        );
    }
}
