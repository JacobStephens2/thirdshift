//! Read Muse's retained session records for the last reply and token usage.

use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(in crate::harness) struct SessionLog {
    pub(in crate::harness) message: Option<String>,
    pub(in crate::harness) models: Vec<String>,
    tokens: Option<Tokens>,
}

#[derive(Default)]
struct Tokens {
    input: u64,
    cached: u64,
    output: u64,
}

impl SessionLog {
    pub fn parse(text: &str, session_id: &str) -> Result<Self> {
        let mut log = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let record: Value = serde_json::from_str(line)?;
            if let Some(children) = record["children"].as_array() {
                for child in children {
                    let record: Value =
                        serde_json::from_str(child["record_json"].as_str().unwrap_or(""))?;
                    log.record(&record, session_id);
                }
            } else {
                log.record(&record, session_id);
            }
        }
        Ok(log)
    }
    fn record(&mut self, record: &Value, session_id: &str) {
        if record["stream"]["id"].as_str() != Some(session_id) || record["payload"]["kind"] != "run"
        {
            return;
        }
        let event = &record["payload"]["event"];
        match event["kind"].as_str() {
            Some("assistant_message_committed") => {
                self.message = event["text"].as_str().map(String::from);
            }
            Some("model_completed") => {
                if let Some(model) = event["model"].as_str().filter(|model| !model.is_empty())
                    && self.models.last().map(String::as_str) != Some(model)
                {
                    self.models.push(model.to_string());
                }
                if let Some(usage) = event["usage"].as_object() {
                    let tokens = self.tokens.get_or_insert_default();
                    tokens.input += usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    tokens.cached += usage
                        .get("cached_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    tokens.output += usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    pub(in crate::harness) fn summary(&self) -> Option<String> {
        self.tokens.as_ref().map(|tokens| {
            format!(
                "{} input tokens ({} cached), {} output tokens",
                tokens.input, tokens.cached, tokens.output
            )
        })
    }
}

/// Date folders are year/month/day; never descend into sub-agent logs.
fn find_log(root: &Path, id: &str, depth: usize) -> Option<PathBuf> {
    let log = root.join(id).join("session.jsonl");
    if log.is_file() {
        return Some(log);
    }
    if depth == 0 {
        return None;
    }
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .find_map(|entry| find_log(&entry.path(), id, depth - 1))
}

pub(in crate::harness) fn read(root: &Path, id: &str) -> Option<SessionLog> {
    if id.contains(['/', '\\']) || id == ".." {
        return None;
    }
    let path = find_log(&root.join("sessions"), id, 3)?;
    SessionLog::parse(&fs::read_to_string(path).ok()?, id).ok()
}
