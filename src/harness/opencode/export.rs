//! Read OpenCode 2's standalone export, whose assistant content and usage
//! survive dropped stream events. Session totals also count the title call.
use super::OpenCode;
use crate::harness::Adapter;
use crate::harness::interpretation::{Facts, Retained, TurnOutcome};
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(in crate::harness) struct SessionExport {
    pub(in crate::harness) message: Option<String>,
    pub(in crate::harness) summary: Option<String>,
    pub(in crate::harness) failure: Option<String>,
    pub(in crate::harness) models: Vec<String>,
}

/// OpenCode records uncached input and visible output separately from cache
/// and reasoning. Combine them into totals while retaining each subtotal.
#[derive(Default)]
struct Usage {
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
    cost: Option<f64>,
}

impl Usage {
    fn add(&mut self, message: &Value) {
        let tokens = &message["tokens"];
        self.input += tokens["input"].as_u64().unwrap_or(0);
        self.cache_read += tokens["cache"]["read"].as_u64().unwrap_or(0);
        self.cache_write += tokens["cache"]["write"].as_u64().unwrap_or(0);
        self.output += tokens["output"].as_u64().unwrap_or(0);
        self.reasoning += tokens["reasoning"].as_u64().unwrap_or(0);
        if let Some(cost) = message["cost"].as_f64() {
            *self.cost.get_or_insert(0.0) += cost;
        }
    }
    fn summary(&self) -> String {
        let input = self.input + self.cache_read + self.cache_write;
        let output = self.output + self.reasoning;
        let cost = self
            .cost
            .map(|cost| format!(", ${cost:.4}"))
            .unwrap_or_default();
        format!(
            "{input} input tokens ({} cache read, {} cache write), {output} output tokens ({} reasoning){cost}",
            self.cache_read, self.cache_write, self.reasoning
        )
    }
}

impl SessionExport {
    pub fn parse(text: &str) -> Result<Self> {
        let export: Value = serde_json::from_str(text)?;
        let outcome = export["info"]["outcome"]
            .as_str()
            .ok_or_else(|| anyhow!("no OpenCode outcome in session export"))?;
        let messages = export["messages"]
            .as_array()
            .ok_or_else(|| anyhow!("no messages in OpenCode session export"))?;
        let mut result = Self {
            message: None,
            summary: None,
            failure: None,
            models: Vec::new(),
        };
        let mut usage: Option<Usage> = None;
        for message in messages
            .iter()
            .filter(|message| message["type"] == "assistant")
        {
            if let Some(id) = message["model"]["id"].as_str().filter(|id| !id.is_empty()) {
                let model = message["model"]["providerID"]
                    .as_str()
                    .filter(|provider| !provider.is_empty())
                    .map_or_else(|| id.to_string(), |provider| format!("{provider}/{id}"));
                if result.models.last() != Some(&model) {
                    result.models.push(model);
                }
            }
            result.message = message["content"]
                .as_array()
                .and_then(|parts| parts.iter().rev().find(|part| part["type"] == "text"))
                .and_then(|part| part["text"].as_str())
                .map(String::from);
            if message["tokens"].is_object() {
                usage.get_or_insert_default().add(message);
            }
            if let Some(error) = super::error_text(&message["error"]) {
                result.failure = Some(error.into());
            }
        }
        result.summary = usage.as_ref().map(Usage::summary);
        if outcome == "failed" {
            result.failure = Some(
                super::error_text(&export["info"]["error"])
                    .map(String::from)
                    .or(result.failure)
                    .unwrap_or_else(|| "OpenCode's exported outcome is failed".into()),
            );
        } else {
            result.failure = None;
        }
        Ok(result)
    }
}

pub(in crate::harness) fn read(worktree: &Path, id: &str) -> Result<Option<SessionExport>> {
    let output = super::super::process::output(
        &OpenCode,
        Command::new(OpenCode.name())
            .args(["session", "export", "--standalone", id])
            .current_dir(worktree),
        None,
    );
    match output {
        Ok(output) if output.status.success() => {
            Ok(SessionExport::parse(&String::from_utf8_lossy(&output.stdout)).ok())
        }
        Err(error) if crate::interrupt::requested() => Err(error),
        _ => Ok(None),
    }
}

/// A readable standalone export owns completion, including empty fields.
pub(super) struct Completion {
    worktree: PathBuf,
}

impl Completion {
    pub(super) fn new(worktree: PathBuf) -> Self {
        Self { worktree }
    }
}

impl Retained for Completion {
    fn reconcile(self: Box<Self>, facts: &mut Facts) -> Result<()> {
        if let Some(id) = facts.ended.session_id.as_deref()
            && let Some(export) = read(&self.worktree, id)?
        {
            facts.ended.final_message = export.message;
            facts.report.summary = export.summary;
            facts.report.models = export.models;
            if export.failure.is_some() {
                facts.outcome = TurnOutcome::Failed;
                facts.diagnostic = facts.diagnostic.take().or(export.failure);
            }
        }
        facts.report.set_model_progress();
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::harness) fn recording_interpretation(
    root: &Path,
    decoder: Box<dyn crate::harness::interpretation::Decoder>,
) -> crate::harness::interpretation::Interpretation {
    crate::harness::interpretation::Interpretation::new("claude", decoder)
        .with_retained(Box::new(Completion::new(root.to_path_buf())))
}
