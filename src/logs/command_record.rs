//! Durable Command facts shared by the recorder and status reader.

use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::Work;
use super::effects::{OnMachine, Outside};
use crate::harness::Choice;
use crate::issue::IssueUrl;

/// A Pass owns its dispatched work in the same process and atomic record.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct CommandTree {
    #[serde(flatten, deserialize_with = "read_command")]
    pub command: CommandRecord,
    #[serde(default)]
    pub dispatched: Vec<CommandRecord>,
}

fn read_command<'de, D: serde::Deserializer<'de>>(reader: D) -> Result<CommandRecord, D::Error> {
    // With arbitrary_precision, Serde's flattened buffer represents fractional
    // numbers as maps. Value restores them before typed metrics read their f64s.
    let value = serde_json::Value::deserialize(reader)?;
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

impl CommandTree {
    pub fn active(&mut self) -> &mut CommandRecord {
        match self.dispatched.last_mut() {
            Some(dispatched) if dispatched.outcome.is_none() => dispatched,
            _ => &mut self.command,
        }
    }
}

/// A human recheck changes the Verdict, without changing when the Command ran.
/// The Experiment's exclusive lock serializes this with completion and rechecks.
pub(crate) fn experiment_rechecked(log: &Path, id: &str, outcome: &str) -> Result<()> {
    let path = log.with_extension("command.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut tree: CommandTree = serde_json::from_slice(&bytes)?;
    ensure!(
        tree.command.kind == "Experiment"
            && tree
                .command
                .experiment
                .as_ref()
                .is_some_and(|experiment| experiment.id == id)
            && tree.command.outcome.is_some(),
        "Command record does not belong to the completed Experiment"
    );
    tree.command.outcome = Some(outcome.into());
    OnMachine.replace_record(&path, &serde_json::to_vec_pretty(&tree)?)
}

#[derive(Clone, Serialize, Deserialize)]
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
    /// Reader-only provenance: legacy retention estimates are not recorded finishes.
    #[serde(skip)]
    pub end_time_estimated: bool,
    pub stages: Vec<Stage>,
    pub session_log: Option<PathBuf>,
    pub outcome: Option<String>,
    #[serde(default)]
    pub run_metrics: Option<super::run_metrics::RunMetrics>,
    pub cause: Option<String>,
    pub issue_branch: Option<String>,
    pub pull_request: Option<String>,
    #[serde(default)]
    pub parent_record: Option<PathBuf>,
    #[serde(default)]
    pub parent_dispatch: Option<String>,
    #[serde(default)]
    pub tickets: Vec<TicketRecord>,
    /// Local knowledge at completion; the Inbox overlays its kept GitHub answer.
    #[serde(default)]
    pub security_findings_waiting: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_findings: Vec<crate::security::RecordedFinding>,
    /// Findings this dispatched Security fix addresses, kept before launch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_fix_findings: Vec<crate::security::RecordedFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experiment: Option<crate::evals::experiment::Description>,
    /// The Custom process name. The selector in `name` stays `process-<name>-<stamp>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
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

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct TicketRecord {
    pub issue: Issue,
    pub state: TicketState,
    pub blockers: Vec<Issue>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TicketState {
    Running,
    Merged,
    Failed,
    Waiting,
    #[serde(rename = "not started")]
    NotStarted,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Stage {
    pub name: String,
    pub began_at: String,
    pub session_log: Option<PathBuf>,
}

/// A retained stage interval, optionally extended to an observation of live work.
pub(super) struct StageTime<'a> {
    pub stage: &'a Stage,
    pub began: Option<DateTime<FixedOffset>>,
    pub ended: Option<DateTime<FixedOffset>>,
}

impl StageTime<'_> {
    pub fn seconds(&self) -> Option<f64> {
        let (began, ended) = self.began.zip(self.ended)?;
        let milliseconds = ended.signed_duration_since(began).num_milliseconds();
        (milliseconds >= 0).then_some(milliseconds as f64 / 1000.0)
    }
}

// Keep typed stages, persisted names and display order in one declaration.
macro_rules! stage_names {
    ($($(#[$meta:meta])* $stage:ident => $name:literal,)*) => {
        /// Internal transitions are typed; persisted names stay tolerant of new stages.
        #[derive(Clone, Copy)]
        pub enum StageName {
            $($(#[$meta])* $stage,)*
        }

        impl StageName {
            pub fn name(self) -> &'static str {
                match self {
                    $(Self::$stage => $name,)*
                }
            }

            /// Declaration order, with unfamiliar persisted names after every known stage.
            pub(super) fn order(name: &str) -> usize {
                match name {
                    $($name => Self::$stage as usize,)*
                    _ => usize::MAX,
                }
            }
        }
    };
}

stage_names! {
    ProcessStep => "Process Step",
    Implement => "implement",
    CodeReview => "Code review",
    Resume => "Resume",
    SecurityReview => "Security review",
    PullRequest => "pull request",
    CiWatch => "CI watch",
    Repair => "Repair",
    SelfMerge => "Self-merge",
    Cleanup => "cleanup",
    ArchitectureReview => "Architecture review",
    Tickets => "Tickets",
    SpecReview => "Spec review",
    SecurityAudit => "Security audit",
    SecurityReproduction => "Security reproduction",
    SecurityFixPublishing => "Security fix publishing",
    /// A fixed finding's proof-of-concept recheck against the merged Base
    /// branch, before its private record is closed.
    SecurityCloseCheck => "Security close check",
    ArmBuilds => "Arm builds",
    EvalTasks => "Eval tasks",
    Trials => "Trials",
    Sampling => "sampling",
    Grading => "grading",
    /// A Pickup run or Security run waiting on the Run it dispatched.
    DispatchedRun => "dispatched Run",
    /// A Pickup run or Security run waiting on the Spec run it dispatched.
    DispatchedSpecRun => "dispatched Spec run",
}

impl StageName {
    /// The stage a pass enters while `work`, the Command it dispatched, runs.
    pub(super) fn dispatched(work: Work<'_>) -> Self {
        match work {
            Work::SpecRun(_) => Self::DispatchedSpecRun,
            Work::Run(_)
            | Work::BaseFix(_)
            | Work::PickupRun(_)
            | Work::ArchitectRun(_)
            | Work::SecurityRun(_)
            | Work::ScoringRun(_)
            | Work::Experiment(_)
            | Work::Process { .. } => Self::DispatchedRun,
        }
    }

    /// Whether the stage named `name` is a pass's dispatch stage. Such a
    /// stage has no Usual range and is never Silent: the dispatched
    /// Command's own stages carry its health.
    pub(super) fn is_dispatch(name: &str) -> bool {
        [Self::DispatchedRun, Self::DispatchedSpecRun]
            .into_iter()
            .any(|stage| stage.name() == name)
    }

    /// Stages entered by Sessions, as distinct from delivery or dispatch work.
    pub(super) fn is_session(name: &str) -> bool {
        [
            Self::Implement,
            Self::Resume,
            Self::Repair,
            Self::SpecReview,
            Self::SecurityReview,
            Self::ArchitectureReview,
            Self::SecurityAudit,
            Self::SecurityReproduction,
            Self::SecurityFixPublishing,
        ]
        .into_iter()
        .any(|stage| stage.name() == name)
    }

    pub(super) fn known(name: &str) -> bool {
        Self::order(name) != usize::MAX
    }
}

impl CommandRecord {
    /// The Custom process's own name. An older record with no `process_name`
    /// falls back to the selector in `name`.
    pub(super) fn process_label(&self) -> Option<&str> {
        (self.kind == "Process").then(|| self.process_name.as_deref().unwrap_or(self.name.as_str()))
    }

    /// `Process <name> on <repo>`, or `Process <name> · <repo>#<n>` when the
    /// record names an issue. Status text adds the kind word itself.
    pub(super) fn process_heading(&self) -> Option<String> {
        let name = self.process_label()?;
        Some(match self.issue.as_ref() {
            Some(issue) => format!("Process {name} · {}#{}", self.repository, issue.number),
            None => format!("Process {name} on {}", self.repository),
        })
    }

    /// The next transition closes a stage; only a recorded outcome closes the
    /// final stage. A live caller can supply its single sampled observation time.
    pub fn stage_times(
        &self,
        live_until: Option<DateTime<FixedOffset>>,
    ) -> impl Iterator<Item = StageTime<'_>> {
        self.stages.iter().enumerate().map(move |(index, stage)| {
            let ended = if let Some(next) = self.stages.get(index + 1) {
                DateTime::parse_from_rfc3339(&next.began_at).ok()
            } else if self.outcome.is_some() {
                self.ended_at
                    .as_deref()
                    .and_then(|end| DateTime::parse_from_rfc3339(end).ok())
            } else {
                live_until
            };
            StageTime {
                stage,
                began: DateTime::parse_from_rfc3339(&stage.began_at).ok(),
                ended,
            }
        })
    }

    /// Exact retained Session references, including earlier stages. Callers
    /// decide whether they need a deduplicated inventory.
    pub fn session_references(&self) -> impl Iterator<Item = &PathBuf> {
        self.stages
            .iter()
            .filter_map(|stage| stage.session_log.as_ref())
            .chain(self.session_log.as_ref())
    }

    pub fn finish(
        &mut self,
        account: &crate::run_ending::Account<'_>,
        outside: &mut impl super::effects::Outside,
        ranges: &super::usual_range::UsualRanges,
        unaddressed_findings: Option<crate::pull_request::UnaddressedFindings>,
    ) {
        self.ended_at = Some(outside.now().to_rfc3339());
        self.outcome = Some(account.outcome.into());
        self.cause = account
            .ended
            .as_ref()
            .err()
            .map(|cause| cause.full().into());
        self.pull_request = account.pr_url.map(String::from);
        self.security_findings_waiting = account.security_findings_waiting;
        self.security_findings = account.security_findings.unwrap_or_default().to_vec();
        if matches!(self.kind.as_str(), "Run" | "Spec run" | "Base fix") {
            self.run_metrics = Some(super::run_metrics::RunMetrics::of(
                self,
                ranges,
                outside,
                unaddressed_findings,
            ));
        }
    }

    pub fn live(&self, outside: &mut impl super::effects::Outside) -> bool {
        self.outcome.is_none()
            && !self.process_start.is_empty()
            && outside.process_start(self.process_id).as_deref() == Some(&self.process_start)
    }

    pub fn new(work: Work<'_>, stamp: &str, harness: &Choice, now: DateTime<FixedOffset>) -> Self {
        let issue = work.issue_url().map(|issue| Issue {
            number: issue.number,
            url: issue.url.clone(),
            // The Ready issue listing named a Pickup run's issue before it started.
            title: match work {
                Work::PickupRun(listed) => Some(listed.title.clone()),
                _ => None,
            },
        });
        let name = match work {
            Work::ArchitectRun(_) => format!("architect-{stamp}"),
            Work::SecurityRun(_) => format!("secure-{stamp}"),
            Work::ScoringRun(_) => format!("scoring-{stamp}"),
            Work::PickupRun(_) => format!("pickup-{stamp}"),
            Work::Experiment(_) => format!("experiment-{stamp}"),
            Work::Process { name, .. } => format!("process-{name}-{stamp}"),
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => {
                format!("{}#{}", issue.repo_slug(), issue.number)
            }
        };
        Self {
            version: 2,
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
            end_time_estimated: false,
            stages: Vec::new(),
            session_log: None,
            outcome: None,
            run_metrics: None,
            cause: None,
            issue_branch: None,
            pull_request: None,
            parent_record: None,
            parent_dispatch: None,
            tickets: Vec::new(),
            security_findings_waiting: false,
            security_findings: Vec::new(),
            security_fix_findings: Vec::new(),
            experiment: None,
            process_name: match work {
                Work::Process { name, .. } => Some(name.to_string()),
                _ => None,
            },
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
