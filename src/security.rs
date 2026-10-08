//! Report-only Security runs: gate before work, audit origin's Base branch,
//! then record the validated findings privately for the Day shift.

use std::fmt;

use anyhow::Result;

use crate::asks::Flags;
use crate::config::UserConfig;
use crate::failed_run::FailedRun;
use crate::github::ListedIssue;
use crate::github::SecurityRecords;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{LaunchAndGitHub, Outside};

pub mod audit;

pub enum Outcome {
    Skipped(Skipped),
    Audited(Result<Recorded, FailedRun>),
}

pub enum Skipped {
    AlreadyRunning(AlreadyRunning),
    ReadyIssue(ListedIssue),
    FindingWaiting,
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
        Err(error) => return Outcome::Audited(Err(error.into())),
    };
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
    )
}

fn run_through(outside: &mut impl Outside, repo: &Repo, base: &str) -> Outcome {
    match outside.ready_issue() {
        Ok(Some(ready)) => {
            let skipped = Skipped::ReadyIssue(ready.listed);
            outside.skipped(Pass::Security, &skipped);
            return Outcome::Skipped(skipped);
        }
        Ok(None) => {}
        Err(error) => return Outcome::Audited(Err(error.into())),
    }
    let records = match outside.security_records() {
        Ok(records) => records,
        Err(error) => return Outcome::Audited(Err(error.into())),
    };
    if records.waiting_for_day_shift() {
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
        Err(error) => return Outcome::Audited(Err(error.into())),
    }
    Outcome::Audited(audit_and_record(outside, repo, base, records))
}

fn audit_and_record(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
    mut known: SecurityRecords,
) -> Result<Recorded, FailedRun> {
    outside.check_harness()?;
    outside.check_node()?;
    outside.started(Work::SecurityRun(repo));
    let (audited, log) = outside.audit(base);
    let recorded = (|| -> Result<Recorded> {
        let audited = audited?;
        let mut recorded = Recorded {
            created: 0,
            existing: 0,
        };
        if audited.findings.is_empty() {
            return Ok(recorded);
        }
        for finding in audited.findings {
            if finding.already_recorded(&known) {
                recorded.existing += 1;
            } else {
                let record = outside.create_security_record(&known, &finding)?;
                known.remember(record);
                recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
            }
        }
        Ok(recorded)
    })();
    recorded.map_err(|error| FailedRun {
        log,
        ..error.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, widgets};

    #[test]
    fn a_draft_without_severity_waits_before_prerequisites_or_audit() {
        let mut outside = InMemory::default()
            .advisories(vec![serde_json::json!({
                "state": "draft", "severity": null,
                "ghsa_id": "GHSA-test", "summary": "Unchecked input size"
            })])
            .harness_failing();
        let outcome = run_through(&mut outside, &widgets(), "main");
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
            let outcome = run_through(&mut outside, &widgets(), "main");
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
            let outcome = run_through(&mut outside, &widgets(), "main");
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
        let outcome = run_through(&mut outside, &widgets(), "main");
        assert!(matches!(outcome, Outcome::Audited(Ok(_))));
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
            run_through(&mut outside, &widgets(), "main"),
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
                    let outcome = run_through(&mut outside, &widgets(), "main");
                    let expected = if ready {
                        "Ready issue #7"
                    } else if waiting {
                        "a Security finding is waiting for the Day shift"
                    } else if unchanged {
                        "Base branch main hasn't changed since the last completed Security audit"
                    } else {
                        assert!(matches!(outcome, Outcome::Audited(Ok(_))));
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
                run_through(&mut outside, &widgets(), "main"),
                Outcome::Audited(Err(_))
            ));
            assert_eq!(outside.calls, expected);
        }
    }

    #[test]
    fn a_failed_audit_records_nothing_and_keeps_its_session_log() {
        let mut outside = InMemory::default()
            .audit_failing("invalid report")
            .session_log("audit.jsonl");
        let Outcome::Audited(Err(failed)) = run_through(&mut outside, &widgets(), "main") else {
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
                json!({"state":"closed", "description":"Fingerprint: `old`"}),
            ]);
        let Outcome::Audited(Ok(recorded)) = run_through(&mut outside, &widgets(), "main") else {
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
    }
}
