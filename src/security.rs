//! Security runs: gate before work, audit origin's Base branch, then record
//! and reproduce the findings privately for the Day shift.

use std::fmt;

use anyhow::Result;

use crate::asks::Flags;
use crate::config::UserConfig;
use crate::failed_run::FailedRun;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{LaunchAndGitHub, Outside};

pub mod audit;
pub mod reproduction;

pub enum Outcome {
    Skipped(Skipped),
    Audited(Result<Recorded, FailedRun>),
}

pub enum Skipped {
    AlreadyRunning(AlreadyRunning),
    ReadyIssue(ListedIssue),
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
    Outcome::Audited(audit_and_record(outside, repo, base))
}

fn audit_and_record(
    outside: &mut impl Outside,
    repo: &Repo,
    base: &str,
) -> Result<Recorded, FailedRun> {
    outside.check_harness()?;
    outside.check_node()?;
    outside.started(Work::SecurityRun(repo));
    let (audited, log) = outside.audit(base);
    let mut log = log;
    let recorded = (|| -> Result<Recorded> {
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
            let record = if let Some(record) = known.finding(&finding)? {
                recorded.existing += 1;
                record
            } else {
                let record = outside.create_security_record(&known, &finding)?;
                let read = known.record(&record)?;
                known.remember(record);
                recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
                read
            };
            if seen.insert(finding.fingerprint) {
                records.push(record);
            }
        }
        for (index, record) in records.iter().enumerate() {
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
            outside.step(format!(
                "Security reproduction {number}: {}",
                reproduced.outcome
            ));
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
    fn checks_prerequisites_then_audits_without_pulling() {
        let mut outside = InMemory::default();
        let outcome = run_through(&mut outside, &widgets(), "main");
        assert!(matches!(outcome, Outcome::Audited(Ok(_))));
        assert_eq!(
            outside.calls,
            vec![
                Call::ReadySearch,
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
    fn each_failed_prerequisite_prevents_later_operations() {
        for (mut outside, expected) in [
            (
                InMemory::default().harness_failing(),
                vec![Call::ReadySearch, Call::HarnessCheck],
            ),
            (
                InMemory::default().node_failing(),
                vec![Call::ReadySearch, Call::HarnessCheck, Call::NodeCheck],
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
        assert!(!outside.calls.contains(&Call::AdvisoryList));
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
                json!({"ghsa_id":"old", "state":"closed", "description":"Fingerprint: `old`"}),
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
                &Call::Reproduce("old".into()),
                &Call::UpdateSecurityRecord("old".into()),
                &Call::Reproduce("new".into()),
                &Call::UpdateSecurityRecord("new".into()),
            ]
        );
    }
}
