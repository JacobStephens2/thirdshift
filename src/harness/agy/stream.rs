//! Antigravity CLI's event envelopes. Final text, failure and cumulative
//! usage come only from the final result, never concatenated text deltas.
use crate::harness::interpretation::{Decoder, Ended, Facts, Report, TurnOutcome};
use crate::progress::{bash, shorten};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Default)]
pub struct AgyProgress {
    session_id: Option<String>,
    tools: HashSet<u64>,
    result: Option<Value>,
}

impl Decoder for AgyProgress {
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
    fn complete(self: Box<Self>) -> Facts {
        let summary = self.summary();
        let result = self.result.as_ref();
        Facts {
            report: Report {
                warnings: Vec::new(),
                summary,
            },
            ended: Ended {
                session_id: self.session_id.clone(),
                // ACTIVE describes a tool step, not killed background work.
                killed: Vec::new(),
                final_message: result
                    .and_then(|result| result["response"].as_str())
                    .map(String::from),
            },
            outcome: TurnOutcome::from_failed(
                result.is_some_and(|result| result["status"] == "ERROR"),
            ),
            diagnostic: self.error().map(String::from),
        }
    }
}

impl AgyProgress {
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
    fn error(&self) -> Option<&str> {
        self.result.as_ref()?["error"]
            .as_str()
            .filter(|error| !error.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };

    const INIT: &str = r#"{"event":"init","conversation_id":"recorded-conversation","init":{"cwd":"/work/widgets","permission_mode":"always-proceed"}}"#;
    const TOOL: &str = r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"cargo test"}}}}"#;
    const RESULT: &str = r#"{"event":"result","result":{"conversation_id":"recorded-conversation","status":"SUCCESS","response":"Final reply only.","num_turns":2,"usage":{"input_tokens":12518,"output_tokens":29,"thinking_tokens":28,"cache_read_tokens":200,"total_tokens":12547}}}"#;

    #[test]
    fn recorded_events_give_progress_id_final_reply_and_cumulative_usage() {
        let mut stream = stream(Harness::Agy, "");
        assert_eq!(stream.condense(INIT), ["session started"]);
        assert!(stream.condense(INIT).is_empty());
        assert_eq!(stream.condense(TOOL), ["$ cargo test"]);
        assert!(stream.condense(TOOL).is_empty());
        stream.condense(&TOOL.replace("ACTIVE", "DONE"));
        stream.condense(r#"{"event":"step_update","step_update":{"step_index":3,"step_type":"agent_response","text_delta":"Earlier reply."}}"#);
        stream.condense(RESULT);
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("recorded-conversation")
        );
        assert_eq!(
            stream.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("Final reply only.")
        );
        assert_eq!(
            stream.report.as_ref().unwrap().summary.clone().as_deref(),
            Some(
                "2 turns, 12518 input tokens (200 cached), 29 output tokens (28 thinking), 12547 total tokens"
            )
        );
        assert!(stream.outcome.is_ok());

        assert!(stream.outcome.as_ref().unwrap().killed_work().is_empty());
    }

    #[test]
    fn the_last_result_replaces_reply_usage_and_outcome() {
        let mut stream = stream(Harness::Agy, "");
        stream.condense(RESULT);
        let error =
            r#"{"event":"result","result":{"status":"ERROR","error":"interrupted","response":""}}"#;
        assert_eq!(stream.condense(error), ["error: interrupted"]);
        let stream = finish(stream);
        assert!(stream.outcome.is_err());
        assert_eq!(
            stream.outcome.as_ref().unwrap_err().to_string(),
            "agy's turn failed: interrupted"
        );

        assert_eq!(stream.report.as_ref().unwrap().summary.clone(), None);
    }

    #[test]
    fn an_active_tool_step_does_not_prove_background_work_was_killed() {
        let mut stream = stream(Harness::Agy, "");
        stream.condense(TOOL);
        stream.condense(RESULT);
        assert!(finish(stream).outcome.unwrap().killed.is_empty());
    }

    #[test]
    fn unknown_and_malformed_lines_are_ignored() {
        let mut stream = stream(Harness::Agy, "");
        for line in [
            "not JSON",
            "{}",
            r#"{"event":"future"}"#,
            r#"{"event":"step_update","step_update":{}}"#,
        ] {
            assert!(stream.condense(line).is_empty());
        }
        let stream = finish(stream);
        assert_eq!(stream.outcome.as_ref().unwrap().session_id.as_deref(), None);
        assert_eq!(stream.report.as_ref().unwrap().summary.clone(), None);
    }
}
