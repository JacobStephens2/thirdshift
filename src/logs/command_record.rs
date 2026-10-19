//! Durable Command facts shared by the recorder and status reader.

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::Work;
use crate::harness::Choice;
use crate::issue::IssueUrl;

/// A Pass owns its dispatched work in the same process and atomic record.
#[derive(Serialize, Deserialize)]
pub(super) struct CommandTree {
    #[serde(flatten)]
    pub command: CommandRecord,
    #[serde(default)]
    pub dispatched: Vec<CommandRecord>,
}

impl CommandTree {
    pub fn active(&mut self) -> &mut CommandRecord {
        match self.dispatched.last_mut() {
            Some(dispatched) if dispatched.outcome.is_none() => dispatched,
            _ => &mut self.command,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct CommandRecord {
    pub version: u32,
    pub kind: String,
    pub repository: String,
    #[serde(default)]
    pub name: String,
    pub issue: Option<Issue>,
    pub hostname: String,
    pub process_id: u32,
    pub process_start: String,
    pub harness: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    pub stages: Vec<Stage>,
    pub session_log: Option<PathBuf>,
    pub outcome: Option<String>,
    pub cause: Option<String>,
    pub issue_branch: Option<String>,
    pub pull_request: Option<String>,
    #[serde(default)]
    pub parent_record: Option<PathBuf>,
    #[serde(default)]
    pub parent_dispatch: Option<String>,
    #[serde(default)]
    pub tickets: Vec<TicketRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
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
    ArchitectureReview,
    Tickets,
    SpecReview,
    SecurityAudit,
    SecurityReproduction,
    SecurityFixPublishing,
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
            Self::ArchitectureReview => "Architecture review",
            Self::Tickets => "Tickets",
            Self::SpecReview => "Spec review",
            Self::SecurityAudit => "Security audit",
            Self::SecurityReproduction => "Security reproduction",
            Self::SecurityFixPublishing => "Security fix publishing",
        }
    }

    pub(super) fn known(name: &str) -> bool {
        [
            Self::Implement,
            Self::Resume,
            Self::SecurityReview,
            Self::PullRequest,
            Self::CiWatch,
            Self::Repair,
            Self::SelfMerge,
            Self::Cleanup,
            Self::ArchitectureReview,
            Self::Tickets,
            Self::SpecReview,
            Self::SecurityAudit,
            Self::SecurityReproduction,
            Self::SecurityFixPublishing,
        ]
        .into_iter()
        .any(|stage| stage.name() == name)
    }
}

impl CommandRecord {
    /// Exact retained Session references, including earlier stages. Callers
    /// decide whether they need a deduplicated inventory.
    pub fn session_references(&self) -> impl Iterator<Item = &PathBuf> {
        self.stages
            .iter()
            .filter_map(|stage| stage.session_log.as_ref())
            .chain(self.session_log.as_ref())
    }

    pub fn finish(&mut self, account: &crate::run_ending::Account<'_>, now: DateTime<FixedOffset>) {
        self.ended_at = Some(now.to_rfc3339());
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

    pub fn new(work: Work<'_>, stamp: &str, harness: &Choice, now: DateTime<FixedOffset>) -> Self {
        let issue = match work {
            Work::Run(issue)
            | Work::BaseFix(issue)
            | Work::SpecRun(issue)
            | Work::PickupRun(issue) => Some(Issue {
                number: issue.number,
                url: issue.url.clone(),
                title: None,
            }),
            Work::ArchitectRun(_) | Work::SecurityRun(_) => None,
        };
        let name = match work {
            Work::ArchitectRun(_) => format!("architect-{stamp}"),
            Work::SecurityRun(_) => format!("secure-{stamp}"),
            Work::PickupRun(_) => format!("pickup-{stamp}"),
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => {
                format!("{}#{}", issue.repo_slug(), issue.number)
            }
        };
        Self {
            version: 1,
            kind: if matches!(work, Work::BaseFix(_)) {
                "Base fix".into()
            } else {
                work.kind().to_string()
            },
            repository: work.repo().slug(),
            name,
            issue,
            hostname: String::new(),
            process_id: std::process::id(),
            process_start: String::new(),
            harness: harness.harness.name().into(),
            model: harness.model.clone(),
            effort: harness.effort.clone(),
            started_at: now.to_rfc3339(),
            ended_at: None,
            stages: Vec::new(),
            session_log: None,
            outcome: None,
            cause: None,
            issue_branch: None,
            pull_request: None,
            parent_record: None,
            parent_dispatch: None,
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
