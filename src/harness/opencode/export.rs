//! Read OpenCode 2's standalone export, whose assistant content and usage
//! survive dropped stream events. Session totals also count the title call.
use super::OpenCode;
use crate::harness::Adapter;
use crate::progress::Stream;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct SessionExport {
    message: Option<String>,
    summary: Option<String>,
    failure: Option<String>,
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
        };
        let mut usage: Option<Usage> = None;
        for message in messages
            .iter()
            .filter(|message| message["type"] == "assistant")
        {
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

pub fn read_after_exit(worktree: &Path, stream: Box<dyn Stream>) -> Box<dyn Stream> {
    let export = stream.session_id().and_then(|id| {
        let output = Command::new(OpenCode.name())
            .args(["session", "export", "--standalone", id])
            .envs(OpenCode.environment().iter().copied())
            .current_dir(worktree)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        SessionExport::parse(&String::from_utf8_lossy(&output.stdout)).ok()
    });
    Box::new(AfterExit { stream, export })
}

struct AfterExit {
    stream: Box<dyn Stream>,
    export: Option<SessionExport>,
}
impl Stream for AfterExit {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        self.stream.condense(raw)
    }
    fn summary(&self) -> Option<String> {
        self.export
            .as_ref()
            .and_then(|export| export.summary.clone())
    }
    fn final_message(&self) -> Option<&str> {
        match &self.export {
            Some(export) => export.message.as_deref(),
            None => self.stream.final_message(),
        }
    }
    fn session_id(&self) -> Option<&str> {
        self.stream.session_id()
    }
    fn killed_background_work(&self) -> Vec<&str> {
        if self.failed() {
            Vec::new()
        } else {
            self.stream.killed_background_work()
        }
    }
    fn failed(&self) -> bool {
        self.stream.failed()
            || self
                .export
                .as_ref()
                .is_some_and(|export| export.failure.is_some())
    }
    fn error(&self) -> Option<&str> {
        self.stream.error().or_else(|| {
            self.export
                .as_ref()
                .and_then(|export| export.failure.as_deref())
        })
    }
    fn warnings(&self) -> Vec<String> {
        self.stream.warnings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_assistant_text_and_summed_usage_exclude_the_title_call_and_user() {
        // OpenCode 2.0.24's exported message format, with irrelevant fields removed.
        let export = SessionExport::parse(r#"{
            "info":{"outcome":"succeeded","tokens":{"input":999999},"cost":9},
            "messages":[
                {"type":"user","text":"prompt","tokens":{"input":999999}},
                {"type":"assistant","tokens":{"input":31,"output":37,"reasoning":158,"cache":{"read":7840,"write":20}},"cost":0.004,"content":[{"type":"text","text":"earlier reply"}]},
                {"type":"assistant","tokens":{"input":39,"output":11,"reasoning":42,"cache":{"read":496,"write":7}},"cost":0.002,"content":[{"type":"text","text":"first part"},{"type":"reasoning","text":"private reasoning"},{"type":"text","text":"Architecture review idea: https://github.com/acme/widgets/issues/8"}]},
                {"type":"idle","outcome":"succeeded"}
            ]
        }"#).unwrap();
        assert_eq!(
            export.message.as_deref(),
            Some("Architecture review idea: https://github.com/acme/widgets/issues/8")
        );
        assert_eq!(
            export.summary.as_deref(),
            Some(
                "8433 input tokens (8336 cache read, 27 cache write), 248 output tokens (200 reasoning), $0.0060"
            )
        );
        assert!(export.failure.is_none());
    }

    #[test]
    fn a_failed_export_reports_the_assistants_error_when_the_session_has_none() {
        let export = SessionExport::parse(r#"{"info":{"outcome":"failed"},"messages":[{"type":"assistant","content":[],"error":{"type":"provider.no-route","message":"Variant unavailable"}}]}"#).unwrap();
        assert_eq!(export.failure.as_deref(), Some("Variant unavailable"));
    }

    #[test]
    fn incomplete_or_malformed_exports_are_unreadable() {
        for text in ["{", "{}", r#"{"info":{"outcome":"succeeded"}}"#] {
            assert!(SessionExport::parse(text).is_err());
        }
    }

    #[test]
    fn an_assistant_without_text_does_not_reuse_an_earlier_reply() {
        let export = SessionExport::parse(r#"{"info":{"outcome":"succeeded"},"messages":[{"type":"assistant","content":[{"type":"text","text":"earlier reply"}]},{"type":"assistant","content":[{"type":"tool","name":"bash"}]}]}"#).unwrap();
        assert!(export.message.is_none());
        assert!(export.summary.is_none());
    }
}
