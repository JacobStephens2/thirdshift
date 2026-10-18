//! Durable Command facts shared by the recorder and status reader.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::harness::Choice;
use crate::issue::IssueUrl;

#[derive(Serialize, Deserialize)]
pub(super) struct CommandRecord {
    pub version: u32,
    pub kind: String,
    pub repository: String,
    pub issue: Issue,
    pub hostname: String,
    pub process_id: u32,
    pub process_start: String,
    pub harness: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub started_at: String,
    pub stages: Vec<Stage>,
    pub session_log: Option<PathBuf>,
    pub outcome: Option<String>,
    pub cause: Option<String>,
    pub issue_branch: Option<String>,
    pub pull_request: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Issue {
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Stage {
    pub name: String,
    pub began_at: String,
    pub session_log: Option<PathBuf>,
}

impl CommandRecord {
    pub fn live(&self, outside: &mut impl super::effects::Outside) -> bool {
        self.outcome.is_none()
            && !self.process_start.is_empty()
            && outside.process_start(self.process_id).as_deref() == Some(&self.process_start)
    }

    pub fn new(issue: &IssueUrl, harness: &Choice, now: DateTime<FixedOffset>) -> Self {
        Self {
            version: 1,
            kind: "Run".into(),
            repository: issue.repo_slug(),
            issue: Issue {
                number: issue.number,
                url: issue.url.clone(),
                title: None,
            },
            hostname: String::new(),
            process_id: std::process::id(),
            process_start: String::new(),
            harness: harness.harness.name().into(),
            model: harness.model.clone(),
            effort: harness.effort.clone(),
            started_at: now.to_rfc3339(),
            stages: Vec::new(),
            session_log: None,
            outcome: None,
            cause: None,
            issue_branch: None,
            pull_request: None,
        }
    }

    /// Unknown stages are deliberately not used to interpret a newer record.
    pub fn current_stage(&self) -> Option<&Stage> {
        self.stages
            .iter()
            .rev()
            .find(|stage| known_stage(&stage.name))
    }
}

pub(super) fn known_stage(name: &str) -> bool {
    matches!(
        name,
        "implement"
            | "Resume"
            | "Security review"
            | "pull request"
            | "CI watch"
            | "Repair"
            | "Self-merge"
            | "cleanup"
    )
}
