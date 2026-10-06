//! Read Muse's retained session records for the last reply and token usage.

use crate::progress::Stream;
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct SessionLog {
    message: Option<String>,
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
    fn summary(&self) -> Option<String> {
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

pub fn read_after_exit(stream: Box<dyn Stream>) -> Box<dyn Stream> {
    let log = stream
        .session_id()
        .filter(|id| !id.contains(['/', '\\']) && *id != "..")
        .and_then(|id| {
            let dir = super::data_dir()?.join("sessions");
            let path = find_log(&dir, id, 3)?;
            SessionLog::parse(&fs::read_to_string(path).ok()?, id).ok()
        });
    Box::new(AfterExit { stream, log })
}

struct AfterExit {
    stream: Box<dyn Stream>,
    log: Option<SessionLog>,
}

impl Stream for AfterExit {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        self.stream.condense(raw)
    }
    fn summary(&self) -> Option<String> {
        self.log.as_ref().and_then(SessionLog::summary)
    }
    fn final_message(&self) -> Option<&str> {
        self.log
            .as_ref()
            .and_then(|log| log.message.as_deref())
            .or_else(|| self.stream.final_message())
    }
    fn session_id(&self) -> Option<&str> {
        self.stream.session_id()
    }
    fn killed_background_work(&self) -> Vec<&str> {
        self.stream.killed_background_work()
    }
    fn failed(&self) -> bool {
        self.stream.failed()
    }
    fn error(&self) -> Option<&str> {
        self.stream.error()
    }
    fn warnings(&self) -> Vec<String> {
        self.stream.warnings()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Retained records observed in Muse 1.4.2, trimmed to the fields read.
    fn record(event: Value) -> Value {
        json!({"stream":{"kind":"session","id":"s1"},"payload_type":"runtime.session",
               "payload":{"kind":"run","event":event}})
    }

    #[test]
    fn the_last_reply_is_kept_and_usage_is_summed_without_child_session_usage() {
        let lines = [
            record(json!({"kind":"assistant_message_committed","text":"first reply"})),
            record(json!({"kind":"model_completed","usage":{"input_tokens":26000,"cached_tokens":12000,"output_tokens":40}})),
            json!({"stream":{"id":"child"},"payload":{"kind":"run","event":{"kind":"model_completed","usage":{"input_tokens":999999}}}}),
            record(json!({"kind":"model_completed","usage":{"input_tokens":24000,"cached_tokens":20000,"output_tokens":30}})),
            record(json!({"kind":"assistant_message_committed","text":"Architecture review idea: https://github.com/acme/widgets/issues/8"})),
        ].map(|line| line.to_string()).join("\n");
        let log = SessionLog::parse(&lines, "s1").unwrap();
        assert_eq!(
            log.message.as_deref(),
            Some("Architecture review idea: https://github.com/acme/widgets/issues/8")
        );
        assert_eq!(
            log.summary().as_deref(),
            Some("50000 input tokens (32000 cached), 70 output tokens")
        );
    }

    #[test]
    fn retained_permission_transactions_are_unwrapped() {
        let message = record(json!({"kind":"assistant_message_committed","text":"last reply"}));
        let frame = json!({"retained_frame":"session_permission_transaction","children":[{"record_json":message.to_string()}]});
        let log = SessionLog::parse(&frame.to_string(), "s1").unwrap();
        assert_eq!(log.message.as_deref(), Some("last reply"));
        assert_eq!(log.summary(), None);
    }

    #[test]
    fn a_truncated_or_non_json_log_is_unreadable() {
        for text in [
            "not JSON",
            "{",
            "{\"children\":[{\"record_json\":\"bad\"}]}",
        ] {
            assert!(SessionLog::parse(text, "s1").is_err());
        }
    }
}
