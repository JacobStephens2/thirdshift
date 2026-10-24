//! Interpret a Harness session once, including optional retained records.

use std::fmt;
use std::path::Path;
use std::process::{ExitStatus, Output};

use anyhow::{Result, anyhow};

use super::said;
use crate::interrupt;

mod stream;
pub(super) use stream::Stream;
pub mod event;
#[cfg(test)]
mod event_tests;
pub(crate) mod limits;
mod models;
mod security;
pub use security::SafeguardRefusal;
use security::Security;

/// Live interpretation has only line condensation. Consume it after the
/// process owner has stopped or waited for the child and joined its workers.
pub struct Interpretation {
    cli: &'static str,
    decoder: Box<dyn Decoder>,
    retained: Option<Box<dyn Retained>>,
    security: Option<Security>,
    models: models::Models,
    events: Option<Vec<event::Event>>,
    work_started: bool,
    plan_limit_reached: bool,
}

/// Reporting survives ordinary failures; only success supplies Resume facts.
pub struct Completion {
    pub report: Option<Report>,
    pub outcome: Result<Ended>,
}

#[derive(Default)]
pub struct Report {
    /// Raw Harness records recovered after exit, in their original encoding.
    pub retained_records: Vec<RetainedRecord>,
    /// Normalized timeline captured by `decode`, enriched by retained records.
    /// Progress-only callers receive only facts recovered at completion.
    pub events: Vec<event::Event>,
    /// Ordered progress supplied after retained completion.
    pub completion_progress: Vec<String>,
    pub warnings: Vec<String>,
    pub summary: Option<String>,
    /// Models reported by session output or retained records.
    pub models: Vec<String>,
}

/// A recovered Harness event keeps its raw body paired with the facts decoded
/// from it, so recording retains the event's clock and call association.
pub struct RetainedRecord {
    pub raw: String,
    pub events: Vec<event::Event>,
}

impl Report {
    /// Retained adapters opt into completion Model progress after reconciliation.
    pub(super) fn set_model_progress(&mut self) {
        self.completion_progress = self
            .models
            .iter()
            .map(|model| format!("Model: {model}"))
            .collect();
    }
}

#[derive(Debug)]
pub struct Ended {
    pub session_id: Option<String>,
    pub killed: Vec<String>,
    pub final_message: Option<String>,
}

impl Ended {
    pub fn killed_work(&self) -> Vec<&str> {
        self.killed.iter().map(String::as_str).collect()
    }
}

/// Each real protocol must supply its outcome explicitly. Streams with no
/// terminal outcome remain accepted, as before, unless execution failed.
pub(super) enum TurnOutcome {
    NotFailed,
    Failed,
}

impl TurnOutcome {
    pub(super) fn from_failed(failed: bool) -> Self {
        if failed {
            Self::Failed
        } else {
            Self::NotFailed
        }
    }
}

pub(super) struct Facts {
    pub report: Report,
    pub ended: Ended,
    pub outcome: TurnOutcome,
    /// May be a retried diagnostic even when the turn did not fail.
    pub diagnostic: Option<String>,
}

pub(super) trait Decoder: Send {
    fn condense(&mut self, raw: &str) -> StreamUpdate;
    fn complete(self: Box<Self>) -> Facts;
    fn limit_records(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn set_home(&mut self, _home: &Path) {}
}

/// Live protocol knowledge stays in the decoder, including evidence from
/// events its progress or completion guards otherwise ignore.
#[derive(Default)]
pub(super) struct StreamUpdate {
    pub progress: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub events: Vec<event::Event>,
}

/// One decoded line. Consumers needing normalized facts use this interface;
/// progress-only callers keep using `condense`, with no event buffering.
pub struct Decoded {
    pub progress: Vec<String>,
    pub events: Vec<event::Event>,
}

pub(super) enum Evidence {
    PlanLimit,
    WorkStarted,
    ObservedModel { name: String, scope: ModelScope },
    RequestedModelFallback,
    SafeguardRefusal(SafeguardRefusal),
}

pub(super) enum ModelScope {
    MainLoop,
    AttributionOnly,
}

/// Optional retained reconciliation belongs to the supplying Harness adapter.
/// Owned implementations follow Interpretation into the transcript reader.
pub(super) trait Retained: Send {
    fn reconcile(self: Box<Self>, facts: &mut Facts) -> Result<()>;
}

enum Failure {
    Execution(anyhow::Error),
    Rejected {
        status: ExitStatus,
        diagnostic: Option<String>,
        refusal: Option<SafeguardRefusal>,
        before_first_turn: bool,
    },
}

/// A failed session keeps the Harness's diagnostic separate from its exit
/// status so callers can report it without parsing the error's display text.
#[derive(Debug)]
pub struct SessionFailure {
    pub harness: &'static str,
    pub diagnostic: Option<String>,
    /// No completed turn, tokens, assistant work or tool calls were observed.
    pub before_first_turn: bool,
    status: ExitStatus,
}

impl fmt::Display for SessionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.harness)?;
        if self.status.success() {
            f.write_str("'s turn failed")?;
        } else {
            write!(
                f,
                " exited {}",
                self.status
                    .code()
                    .map_or("by signal".to_string(), |code| code.to_string())
            )?;
        }
        if let Some(diagnostic) = &self.diagnostic {
            write!(f, ": {diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SessionFailure {}

impl Failure {
    fn session_error(self, cli: &'static str) -> anyhow::Error {
        match self {
            Self::Execution(error) => error,
            Self::Rejected {
                status,
                diagnostic,
                refusal,
                before_first_turn,
            } => {
                let error = anyhow::Error::new(SessionFailure {
                    harness: cli,
                    diagnostic,
                    status,
                    before_first_turn,
                });
                match refusal {
                    Some(refusal) => error.context(refusal),
                    None => error,
                }
            }
        }
    }
}

impl Interpretation {
    pub(super) fn new(cli: &'static str, decoder: Box<dyn Decoder>) -> Self {
        Self {
            cli,
            decoder,
            retained: None,
            security: None,
            models: models::Models::default(),
            events: None,
            work_started: false,
            plan_limit_reached: false,
        }
    }

    pub(super) fn with_retained(mut self, retained: Box<dyn Retained>) -> Self {
        self.retained = Some(retained);
        self
    }

    /// Historical replay must not launch a Harness to export today's state.
    /// Callers can still opt into a local retained reader (Codex patch files).
    pub(crate) fn without_retained(mut self) -> Self {
        self.retained = None;
        self
    }

    /// Apply refusal and Model reporting rules independently of the session's log label.
    pub fn for_security(mut self, requested_model: Option<&str>) -> Self {
        self.security = Some(Security::new(requested_model));
        self
    }

    /// Stream evidence already read, even when interruption suppresses
    /// completion reporting and retained-record reads.
    pub fn observed_models(&self) -> Vec<String> {
        self.models.names().to_vec()
    }

    pub fn plan_limit_reached(&self) -> bool {
        self.plan_limit_reached
    }

    pub fn limit_records(&mut self) -> Vec<String> {
        self.decoder.limit_records()
    }

    pub fn set_home(&mut self, home: &Path) {
        self.decoder.set_home(home);
    }

    /// Unknown or malformed lines produce no progress, never an error.
    pub fn condense(&mut self, raw: &str) -> Vec<String> {
        let decoded = self.read(raw);
        drop(decoded.events);
        decoded.progress
    }

    /// Return live facts and capture them for the authoritative completion
    /// timeline. Retained records refine calls by ID instead of adding calls.
    pub fn decode(&mut self, raw: &str) -> Decoded {
        let decoded = self.read(raw);
        event::reconcile(self.events.get_or_insert_default(), decoded.events.clone());
        decoded
    }

    fn read(&mut self, raw: &str) -> Decoded {
        let StreamUpdate {
            mut progress,
            evidence,
            events,
        } = self.decoder.condense(raw);
        self.work_started |= events.iter().any(work_started);
        for fact in evidence {
            if matches!(fact, Evidence::PlanLimit) {
                self.plan_limit_reached = true;
            }
            if matches!(fact, Evidence::WorkStarted) {
                self.work_started = true;
            }
            if let Evidence::ObservedModel { name, .. } = &fact
                && !self.models.include(name)
            {
                continue;
            }
            if let Some(security) = &mut self.security
                && let Some(line) = security.apply(&fact)
            {
                progress.push(line);
            }
        }
        Decoded { progress, events }
    }

    pub fn finish(self, execution: Result<ExitStatus>) -> Completion {
        let cli = self.cli;
        let (report, outcome) = self.complete(execution);
        Completion {
            report,
            outcome: outcome.map_err(|failure| failure.session_error(cli)),
        }
    }

    /// Captured checks use exactly the same validation and retained policy,
    /// but keep their own refusal context and raw-output fallback.
    pub(super) fn check_output(mut self, output: &Output, refusal: &str) -> Result<()> {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            self.condense(line);
        }
        let (_, outcome) = self.complete(Ok(output.status));
        outcome.map(drop).map_err(|failure| match failure {
            Failure::Execution(error) => error,
            Failure::Rejected { diagnostic, .. } => {
                anyhow!("{refusal}: {}", diagnostic.unwrap_or_else(|| said(output)))
            }
        })
    }

    fn complete(self, execution: Result<ExitStatus>) -> (Option<Report>, Result<Ended, Failure>) {
        if let Err(error) = interrupt::check() {
            return (None, Err(Failure::Execution(error)));
        }
        let mut facts = self.decoder.complete();
        facts.report.events = self.events.unwrap_or_default();
        facts.report.models = self.models.into_names();
        let refusal = self.security.and_then(|security| security.refusal);
        if refusal.is_some() {
            facts.outcome = TurnOutcome::Failed;
        }
        let recovered = self
            .retained
            .map_or(Ok(()), |retained| retained.reconcile(&mut facts));
        if let Err(error) = interrupt::check() {
            return (None, Err(Failure::Execution(error)));
        }
        let outcome = execution
            .and_then(|status| recovered.map(|()| status))
            .map_err(Failure::Execution)
            .and_then(|status| {
                if !status.success() || matches!(facts.outcome, TurnOutcome::Failed) {
                    Err(Failure::Rejected {
                        status,
                        diagnostic: facts.diagnostic,
                        refusal,
                        before_first_turn: !self.work_started
                            && !facts.report.events.iter().any(work_started),
                    })
                } else {
                    Ok(facts.ended)
                }
            });
        (Some(facts.report), outcome)
    }
}

fn work_started(event: &event::Event) -> bool {
    use event::{Event, Outcome};
    let tokens = |usage: &event::Usage| {
        [
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
        ]
        .into_iter()
        .flatten()
        .any(|count| count > 0)
    };
    match event {
        Event::ToolCallStarted { .. } | Event::ToolCallEnded { .. } => true,
        Event::Message { role, text, .. } => role == "assistant" && !text.trim().is_empty(),
        Event::TurnEnded { usage, outcome, .. } => *outcome == Outcome::Succeeded || tokens(usage),
        Event::ModelCallEnded { usage, .. } | Event::Usage { usage, .. } => tokens(usage),
        _ => false,
    }
}
