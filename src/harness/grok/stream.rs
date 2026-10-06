//! Grok's Messages stream shares Claude's framing, with different tool names.

use serde_json::Value;

use crate::harness::claude::stream::ClaudeProgress;
use crate::progress::Stream;

#[derive(Default)]
pub struct GrokProgress {
    claude: ClaudeProgress,
    failed: bool,
    error: Option<String>,
    session_id: Option<String>,
    usage: Option<String>,
    cost_known: bool,
}

impl Stream for GrokProgress {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(mut event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        if let Some(id) = event["session_id"].as_str() {
            self.session_id.get_or_insert_with(|| id.to_string());
        }
        match event["type"].as_str() {
            Some("error") => {
                self.failed = true;
                self.error = event["message"].as_str().map(String::from);
            }
            Some("result") => {
                // Messages streams use zero as a placeholder for unknown cost.
                self.cost_known = event["total_cost_usd"]
                    .as_f64()
                    .is_some_and(|cost| cost > 0.0);
                self.usage = None;
                self.failed = event["is_error"] == true || event["subtype"] != "success";
                let errors: Vec<_> = event["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                if !errors.is_empty() {
                    self.error = Some(errors.join("\n"));
                } else if self.failed
                    && let Some(text) = event["result"].as_str().filter(|text| !text.is_empty())
                {
                    self.error = Some(text.to_string());
                }
                if let (Some(input), Some(output)) = (
                    event["usage"]["input_tokens"].as_u64(),
                    event["usage"]["output_tokens"].as_u64(),
                ) {
                    let cached = event["usage"]["cache_read_input_tokens"]
                        .as_u64()
                        .unwrap_or(0);
                    let created = event["usage"]["cache_creation_input_tokens"]
                        .as_u64()
                        .unwrap_or(0);
                    self.usage = Some(format!(
                        "{input} input tokens, {cached} cache read tokens, {created} cache creation tokens, {output} output tokens"
                    ));
                }
            }
            _ => {}
        }
        if let Some(content) = event
            .get_mut("message")
            .and_then(|message| message.get_mut("content"))
            .and_then(Value::as_array_mut)
        {
            for block in content
                .iter_mut()
                .filter(|block| block["type"] == "tool_use")
            {
                if matches!(
                    block["name"].as_str(),
                    Some("bash" | "run_terminal_command")
                ) {
                    block["name"] = Value::String("Bash".to_string());
                }
                if let Some(path) = block["input"]["path"].as_str().map(String::from) {
                    block["input"]["file_path"] = Value::String(path);
                }
            }
        }
        self.claude.condense(&event.to_string())
    }
    fn summary(&self) -> Option<String> {
        if self.cost_known {
            self.claude.summary().or_else(|| self.usage.clone())
        } else {
            self.usage.clone()
        }
    }
    fn final_message(&self) -> Option<&str> {
        self.claude.final_message()
    }
    fn session_id(&self) -> Option<&str> {
        self.claude.session_id().or(self.session_id.as_deref())
    }
    fn killed_background_work(&self) -> Vec<&str> {
        // Grok documents no Messages frame proving work was killed at exit.
        Vec::new()
    }
    fn failed(&self) -> bool {
        self.failed
    }
    fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_frames_reuse_claude_progress_and_keep_groks_id_final_text_and_spend() {
        let mut stream = GrokProgress::default();
        assert_eq!(stream.condense(r#"{"type":"system","subtype":"init","session_id":"abc123","apiKeySource":"oauth","cwd":"/repo"}"#), ["session started"]);
        assert_eq!(stream.condense(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","input":{"path":"/repo/src/main.rs"}},{"type":"tool_use","name":"bash","input":{"command":"cargo test"}}]}}"#), ["read_file src/main.rs", "$ cargo test"]);
        stream.condense(r#"{"type":"result","subtype":"success","is_error":false,"num_turns":7,"result":"Here's a summary...","total_cost_usd":0.0127,"usage":{"input_tokens":812,"output_tokens":210,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"session_id":"abc123"}"#);
        assert_eq!(stream.session_id(), Some("abc123"));
        assert_eq!(stream.final_message(), Some("Here's a summary..."));
        assert_eq!(stream.summary().as_deref(), Some("7 turns, $0.01"));
        assert!(!stream.failed());
    }

    #[test]
    fn usage_without_a_complete_cost_and_a_result_without_init_are_still_read() {
        let mut stream = GrokProgress::default();
        stream.condense(r#"{"type":"result","subtype":"success","result":"done","num_turns":1,"total_cost_usd":0,"usage":{"input_tokens":812,"output_tokens":210,"cache_read_input_tokens":45,"cache_creation_input_tokens":12},"session_id":"abc123"}"#);
        assert_eq!(stream.session_id(), Some("abc123"));
        assert_eq!(
            stream.summary().as_deref(),
            Some(
                "812 input tokens, 45 cache read tokens, 12 cache creation tokens, 210 output tokens"
            )
        );
    }

    #[test]
    fn malformed_and_unrelated_lines_are_ignored_and_error_frames_report_the_failure() {
        let mut stream = GrokProgress::default();
        for line in [
            "not json",
            "null",
            "[]",
            "{}",
            r#"{"type":"assistant","message":"bad"}"#,
        ] {
            assert!(stream.condense(line).is_empty());
        }
        stream.condense(r#"{"type":"error","message":"backend unavailable"}"#);
        assert!(stream.failed());
        assert_eq!(stream.error(), Some("backend unavailable"));
        stream.condense(r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}"#);
        assert!(stream.failed());
        assert_eq!(stream.error(), Some("quota exceeded"));
    }
}
