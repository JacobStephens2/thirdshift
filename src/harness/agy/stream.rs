//! Antigravity CLI's event envelopes. Final text, failure and cumulative
//! usage come only from the final result, never concatenated text deltas.
use crate::progress::{Stream, bash, shorten};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Default)]
pub struct AgyProgress {
    session_id: Option<String>,
    tools: HashSet<u64>,
    result: Option<Value>,
    running: BTreeMap<u64, String>,
}

impl Stream for AgyProgress {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        match event["event"].as_str() {
            Some("init") if self.session_id.is_none() => {
                self.session_id = event["conversation_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(String::from);
                vec!["session started".to_string()]
            }
            Some("step_update") => {
                let step = &event["step_update"];
                if step["step_type"] != "tool" {
                    return Vec::new();
                }
                let Some(index) = step["step_index"].as_u64() else {
                    return Vec::new();
                };
                if step["state"] == "DONE" {
                    self.running.remove(&index);
                }
                if !self.tools.insert(index) {
                    return Vec::new();
                }
                let name = step["tool_name"].as_str().unwrap_or("tool");
                let input = &step["tool_info"]["parameters"];
                let description = match name {
                    "run_command" => bash(input["CommandLine"].as_str().unwrap_or("")),
                    _ => {
                        let detail = [
                            "AbsolutePath",
                            "TargetFile",
                            "SearchPath",
                            "Query",
                            "Description",
                        ]
                        .iter()
                        .find_map(|key| input[key].as_str());
                        detail.map_or_else(
                            || name.to_string(),
                            |detail| format!("{name} {}", shorten(detail)),
                        )
                    }
                };
                if step["state"] == "ACTIVE" && matches!(name, "run_command" | "invoke_subagent") {
                    self.running.insert(
                        index,
                        description
                            .strip_prefix("$ ")
                            .unwrap_or(&description)
                            .to_string(),
                    );
                }
                vec![description]
            }
            Some("result") => {
                self.result = Some(event["result"].clone());
                self.error()
                    .map(|error| format!("error: {}", shorten(error)))
                    .into_iter()
                    .collect()
            }
            _ => Vec::new(),
        }
    }
    fn summary(&self) -> Option<String> {
        let result = self.result.as_ref()?;
        let usage = &result["usage"];
        Some(format!(
            "{} turns, {} input tokens ({} cached), {} output tokens ({} thinking), {} total tokens",
            result["num_turns"].as_u64()?,
            usage["input_tokens"].as_u64()?,
            usage["cache_read_tokens"].as_u64().unwrap_or(0),
            usage["output_tokens"].as_u64()?,
            usage["thinking_tokens"].as_u64().unwrap_or(0),
            usage["total_tokens"].as_u64()?
        ))
    }
    fn final_message(&self) -> Option<&str> {
        self.result.as_ref()?["response"].as_str()
    }
    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    fn killed_background_work(&self) -> Vec<&str> {
        if self.failed() {
            return Vec::new();
        }
        self.running.values().map(String::as_str).collect()
    }
    fn failed(&self) -> bool {
        self.result
            .as_ref()
            .is_some_and(|result| result["status"] == "ERROR")
    }
    fn error(&self) -> Option<&str> {
        self.result.as_ref()?["error"]
            .as_str()
            .filter(|error| !error.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INIT: &str = r#"{"event":"init","conversation_id":"recorded-conversation","init":{"cwd":"/work/widgets","permission_mode":"always-proceed"}}"#;
    const TOOL: &str = r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"cargo test"}}}}"#;
    const RESULT: &str = r#"{"event":"result","result":{"conversation_id":"recorded-conversation","status":"SUCCESS","response":"Final reply only.","num_turns":2,"usage":{"input_tokens":12518,"output_tokens":29,"thinking_tokens":28,"cache_read_tokens":200,"total_tokens":12547}}}"#;

    #[test]
    fn recorded_events_give_progress_id_final_reply_and_cumulative_usage() {
        let mut stream = AgyProgress::default();
        assert_eq!(stream.condense(INIT), ["session started"]);
        assert!(stream.condense(INIT).is_empty());
        assert_eq!(stream.condense(TOOL), ["$ cargo test"]);
        assert!(stream.condense(TOOL).is_empty());
        stream.condense(&TOOL.replace("ACTIVE", "DONE"));
        stream.condense(r#"{"event":"step_update","step_update":{"step_index":3,"step_type":"agent_response","text_delta":"Earlier reply."}}"#);
        stream.condense(RESULT);
        assert_eq!(stream.session_id(), Some("recorded-conversation"));
        assert_eq!(stream.final_message(), Some("Final reply only."));
        assert_eq!(
            stream.summary().as_deref(),
            Some(
                "2 turns, 12518 input tokens (200 cached), 29 output tokens (28 thinking), 12547 total tokens"
            )
        );
        assert!(!stream.failed());
        assert_eq!(stream.error(), None);
        assert!(stream.killed_background_work().is_empty());
    }

    #[test]
    fn the_last_result_replaces_reply_usage_and_outcome() {
        let mut stream = AgyProgress::default();
        stream.condense(RESULT);
        let error =
            r#"{"event":"result","result":{"status":"ERROR","error":"interrupted","response":""}}"#;
        assert_eq!(stream.condense(error), ["error: interrupted"]);
        assert!(stream.failed());
        assert_eq!(stream.error(), Some("interrupted"));
        assert_eq!(stream.final_message(), Some(""));
        assert_eq!(stream.summary(), None);
    }

    #[test]
    fn unfinished_commands_are_resume_work_only_on_success() {
        let mut stream = AgyProgress::default();
        stream.condense(TOOL);
        stream.condense(RESULT);
        assert_eq!(stream.killed_background_work(), ["cargo test"]);
        stream.condense(r#"{"event":"result","result":{"status":"ERROR","error":"interrupted"}}"#);
        assert!(stream.killed_background_work().is_empty());
    }

    #[test]
    fn unknown_and_malformed_lines_are_ignored() {
        let mut stream = AgyProgress::default();
        for line in [
            "not JSON",
            "{}",
            r#"{"event":"future"}"#,
            r#"{"event":"step_update","step_update":{}}"#,
        ] {
            assert!(stream.condense(line).is_empty());
        }
        assert_eq!(stream.session_id(), None);
        assert_eq!(stream.summary(), None);
    }
}
