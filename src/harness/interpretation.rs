//! Interpret a Harness session once, including optional retained records.

use std::path::PathBuf;
use std::process::{ExitStatus, Output};

use anyhow::{Result, anyhow};

use super::{muse, opencode, said};
use crate::interrupt;

mod stream;
pub(super) use stream::Stream;
mod models;
mod security;
pub use security::SafeguardRefusal;
use security::Security;

/// Live interpretation has only line condensation. Consume it after the
/// process owner has stopped or waited for the child and joined its workers.
pub struct Interpretation {
    cli: &'static str,
    decoder: Box<dyn Decoder>,
    retained: Retained,
    security: Option<Security>,
    models: models::Models,
}

/// Reporting survives ordinary failures; only success supplies Resume facts.
pub struct Completion {
    pub report: Option<Report>,
    pub outcome: Result<Ended>,
}

#[derive(Default)]
pub struct Report {
    pub warnings: Vec<String>,
    pub summary: Option<String>,
    /// Models reported by session output or retained records.
    pub models: Vec<String>,
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
    fn condense(&mut self, raw: &str) -> Vec<String>;
    fn complete(self: Box<Self>) -> Facts;
}

pub(super) enum Retained {
    None,
    Muse(Option<PathBuf>),
    OpenCode(PathBuf),
}

impl Retained {
    fn reconcile(self, facts: &mut Facts) -> Result<()> {
        match self {
            Self::None => {}
            Self::Muse(root) => {
                if let Some(log) = root
                    .as_deref()
                    .zip(facts.ended.session_id.as_deref())
                    .and_then(|(root, id)| muse::log::read(root, id))
                {
                    facts.report.summary = log.summary();
                    if log.message.is_some() {
                        facts.ended.final_message = log.message;
                    }
                    facts.report.models = log.models;
                }
            }
            Self::OpenCode(worktree) => {
                if let Some(id) = facts.ended.session_id.as_deref()
                    && let Some(export) = opencode::export::read(&worktree, id)?
                {
                    // A readable export is authoritative, even without text.
                    facts.ended.final_message = export.message;
                    facts.report.summary = export.summary;
                    facts.report.models = export.models;
                    if export.failure.is_some() {
                        facts.outcome = TurnOutcome::Failed;
                        facts.diagnostic = facts.diagnostic.take().or(export.failure);
                    }
                }
            }
        }
        Ok(())
    }
}

enum Failure {
    Execution(anyhow::Error),
    Rejected {
        status: ExitStatus,
        diagnostic: Option<String>,
        refusal: Option<SafeguardRefusal>,
    },
}

impl Failure {
    fn session_error(self, cli: &str) -> anyhow::Error {
        match self {
            Self::Execution(error) => error,
            Self::Rejected {
                status,
                diagnostic,
                refusal,
            } => {
                let ended = if status.success() {
                    "'s turn failed".to_string()
                } else {
                    format!(
                        " exited {}",
                        status
                            .code()
                            .map_or("by signal".to_string(), |code| code.to_string())
                    )
                };
                let error = match diagnostic {
                    Some(error) => anyhow!("{cli}{ended}: {error}"),
                    None => anyhow!("{cli}{ended}"),
                };
                match refusal {
                    Some(refusal) => error.context(refusal),
                    None => error,
                }
            }
        }
    }
}

impl Interpretation {
    pub(super) fn new(cli: &'static str, decoder: Box<dyn Decoder>, retained: Retained) -> Self {
        Self {
            cli,
            decoder,
            retained,
            security: None,
            models: models::Models::new(cli),
        }
    }

    /// Apply refusal and Model reporting rules independently of the session's log label.
    pub fn for_security(mut self, requested_model: Option<&str>) -> Self {
        self.security = Some(Security::new(self.cli, requested_model));
        self
    }

    /// Stream evidence already read, even when interruption suppresses
    /// completion reporting and retained-record reads.
    pub fn observed_models(&self) -> Vec<String> {
        self.models.names().to_vec()
    }

    /// Unknown or malformed lines produce no progress, never an error.
    pub fn condense(&mut self, raw: &str) -> Vec<String> {
        let mut lines = self.decoder.condense(raw);
        self.models.observe(raw);
        if let Some(security) = &mut self.security
            && let Some(line) = security.observe(raw)
        {
            lines.push(line);
        }
        lines
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
        facts.report.models = self.models.into_names();
        let refusal = self.security.and_then(|security| security.refusal);
        if refusal.is_some() {
            facts.outcome = TurnOutcome::Failed;
        }
        let recovered = self.retained.reconcile(&mut facts);
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
                    })
                } else {
                    Ok(facts.ended)
                }
            });
        (Some(facts.report), outcome)
    }
}
