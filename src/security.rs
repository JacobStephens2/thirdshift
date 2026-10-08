//! Report-only Security runs: gate before work, audit origin's Base branch,
//! then record the validated findings privately for the Day shift.

use std::fmt;

use anyhow::{Context, Result};
use serde_json::Value;

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

pub enum Outcome {
    Skipped(Skipped),
    Audited(Ended),
}

/// The audit's outcome and the private records it reached, including before a failure.
pub struct Ended {
    pub outcome: Result<Recorded, FailedRun>,
    pub findings: Vec<RecordedFinding>,
}

impl From<anyhow::Error> for Ended {
    fn from(error: anyhow::Error) -> Self {
        Self {
            outcome: Err(error.into()),
            findings: Vec::new(),
        }
    }
}

/// Only the metadata allowed in a Run notification; no private write-up.
#[derive(Debug, PartialEq, Eq)]
pub struct RecordedFinding {
    pub severity: Option<String>,
    pub title: String,
    pub url: String,
}

impl RecordedFinding {
    fn of_record(record: &Value) -> Result<Self> {
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
        Err(error) => return Outcome::Audited(error.into()),
    }
    Outcome::Audited(audit_and_record(outside, repo, base))
}

fn audit_and_record(outside: &mut impl Outside, repo: &Repo, base: &str) -> Ended {
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
        for finding in audited.findings {
            let record = if let Some(record) = finding.recorded_in(&known) {
                recorded.existing += 1;
                record
            } else {
                let record = outside.create_security_record(&known, &finding)?;
                recorded.created += 1;
                outside.step("recorded a Security finding privately".to_string());
                known.remember(record)
            };
            findings.push(RecordedFinding::of_record(record)?);
        }
        Ok(recorded)
    })();
    Ended {
        outcome: recorded.map_err(|error| FailedRun {
            log,
            ..error.into()
        }),
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{Call, InMemory, widgets};

    #[test]
    fn checks_prerequisites_then_audits_without_pulling() {
        let mut outside = InMemory::default();
        let outcome = run_through(&mut outside, &widgets(), "main");
        assert!(matches!(
            outcome,
            Outcome::Audited(Ended { outcome: Ok(_), .. })
        ));
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
        }) = run_through(&mut outside, &widgets(), "main")
        else {
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
                json!({"state":"closed", "description":"Fingerprint: `old`", "summary":"Candidate", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-old"}),
            ]);
        let Outcome::Audited(Ended {
            outcome: Ok(recorded),
            ..
        }) = run_through(&mut outside, &widgets(), "main")
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
    }
}
