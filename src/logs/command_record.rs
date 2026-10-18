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
    #[serde(default)]
    pub parent_record: Option<PathBuf>,
    #[serde(default)]
    pub tickets: Vec<TicketRecord>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Issue {
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
}

impl From<&IssueUrl> for Issue {
    fn from(issue: &IssueUrl) -> Self {
        Self {
            number: issue.number,
            url: issue.url.clone(),
            title: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct TicketRecord {
    pub issue: Issue,
    pub state: TicketState,
    pub blockers: Vec<Issue>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TicketState {
    Running,
    Merged,
    Failed,
    Waiting,
    #[serde(rename = "not started")]
    NotStarted,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Stage {
    pub name: String,
    pub began_at: String,
    pub session_log: Option<PathBuf>,
}

/// Internal transitions are typed; persisted names stay tolerant of new stages.
#[derive(Clone, Copy)]
pub enum StageName {
    Implement,
    Resume,
    SecurityReview,
    PullRequest,
    CiWatch,
    Repair,
    SelfMerge,
    Cleanup,
    Tickets,
    SpecReview,
}

impl StageName {
    pub fn name(self) -> &'static str {
        match self {
            Self::Implement => "implement",
            Self::Resume => "Resume",
            Self::SecurityReview => "Security review",
            Self::PullRequest => "pull request",
            Self::CiWatch => "CI watch",
            Self::Repair => "Repair",
            Self::SelfMerge => "Self-merge",
            Self::Cleanup => "cleanup",
            Self::Tickets => "Tickets",
            Self::SpecReview => "Spec review",
        }
    }

    fn known(name: &str) -> bool {
        [
            Self::Implement,
            Self::Resume,
            Self::SecurityReview,
            Self::PullRequest,
            Self::CiWatch,
            Self::Repair,
            Self::SelfMerge,
            Self::Cleanup,
            Self::Tickets,
            Self::SpecReview,
        ]
        .into_iter()
        .any(|stage| stage.name() == name)
    }
}

impl CommandRecord {
    pub fn finish(&mut self, account: &crate::run_ending::Account<'_>) {
        self.outcome = Some(account.outcome.into());
        self.cause = account
            .ended
            .as_ref()
            .err()
            .map(|cause| cause.full().into());
        self.pull_request = account.pr_url.map(String::from);
    }

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
            parent_record: None,
            tickets: Vec::new(),
        }
    }

    /// Unknown stages are deliberately not used to interpret a newer record.
    pub fn current_stage(&self) -> Option<&Stage> {
        self.stages
            .iter()
            .rev()
            .find(|stage| StageName::known(&stage.name))
    }
}
