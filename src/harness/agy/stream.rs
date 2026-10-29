//! Antigravity CLI's event envelopes. Final text, failure and cumulative
//! usage come only from the final result, never concatenated text deltas.
use crate::harness::interpretation::{
    Decoder, Ended, Evidence, Facts, ModelScope, Report, StreamUpdate, TurnOutcome,
};
use crate::progress::{bash, shorten};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Default)]
pub struct AgyProgress {
    session_id: Option<String>,
    tools: HashSet<u64>,
    result: Option<Value>,
    events: super::events::Events,
}

impl AgyProgress {
    fn progress(&mut self, event: &Value) -> Vec<String> {
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
}

impl Decoder for AgyProgress {
    fn condense(&mut self, raw: &str) -> StreamUpdate {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return StreamUpdate::default();
        };
        let mut evidence = Vec::new();
        if event["event"] == "result"
            && event["result"]["num_turns"]
                .as_u64()
                .is_some_and(|turns| turns > 0)
        {
            evidence.push(Evidence::WorkStarted);
        }
        if event["event"] == "init"
            && let Some(name) = event["init"]["model"].as_str()
        {
            let scope = if event["parent_tool_use_id"].is_null() {
                ModelScope::MainLoop
            } else {
                ModelScope::AttributionOnly
            };
            evidence.push(Evidence::ObservedModel {
                name: name.into(),
                scope,
            });
        }
        StreamUpdate {
            events: self.events.decode(&event),
            progress: self.progress(&event),
            evidence,
        }
    }

    fn complete(self: Box<Self>) -> Facts {
        let summary = self.summary();
        let result = self.result.as_ref();
        Facts {
            report: Report {
                warnings: Vec::new(),
                summary,
                ..Report::default()
            },
            ended: Ended {
                session_id: self.session_id.clone(),
                // ACTIVE describes a tool step, not killed background work.
                killed: Vec::new(),
                final_message: result
                    .and_then(|result| result["response"].as_str())
                    .map(String::from),
                completed: result.is_some(),
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

    #[test]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the test picks out the events it asserts on; the other events are not under test"
    )]
    fn review_agy_terminal_result_does_not_repeat_streamed_text() {
        use crate::harness::interpretation::event::Event;
        let mut interpretation = stream(Harness::Agy, "");
        for line in [
            r#"{"event":"init","conversation_id":"c","init":{"model":"gemini"}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c","step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"First part. "}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c","step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"Second part.","usage":{"input_tokens":10,"output_tokens":4}}}"#,
            r#"{"event":"result","result":{"status":"SUCCESS","response":"First part. Second part."}}"#,
        ] {
            interpretation.decode(line);
        }
        let events = finish(interpretation).report.unwrap().events;
        let text: String = events
            .iter()
            .filter_map(|event| match event {
                Event::Message { role, text, .. } if role == "assistant" => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "First part. Second part.");
    }
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };

    const INIT: &str = r#"{"event":"init","conversation_id":"recorded-conversation","init":{"cwd":"/work/widgets","permission_mode":"always-proceed"}}"#;
    const TOOL: &str = r#"{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"cargo test"}}}}"#;
    const RESULT: &str = r#"{"event":"result","result":{"conversation_id":"recorded-conversation","status":"SUCCESS","response":"Final reply only.","num_turns":2,"usage":{"input_tokens":12518,"output_tokens":29,"thinking_tokens":28,"cache_read_tokens":200,"total_tokens":12547}}}"#;

    #[test]
    fn normalized_agy_steps_keep_both_edges_and_ignore_repeated_updates() {
        use crate::harness::interpretation::event::{Event, Outcome};
        let mut stream = stream(Harness::Agy, "");
        for line in [INIT, TOOL, TOOL, &TOOL.replace("ACTIVE", "DONE"), RESULT] {
            stream.decode(line);
        }
        let events = finish(stream).report.unwrap().events;
        assert!(
            matches!(&events[0], Event::ToolCallStarted { tool, input, .. }
            if tool == "run_command" && input["CommandLine"] == "cargo test")
        );
        assert!(matches!(
            &events[1],
            Event::ToolCallEnded {
                outcome: Outcome::Succeeded,
                ..
            }
        ));
        assert!(matches!(
            (&events[0], &events[1]),
            (Event::ToolCallStarted { id: start, .. }, Event::ToolCallEnded { id: end, .. })
                if start == end
        ));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::ToolCallStarted { .. }))
                .count(),
            1
        );
    }

    #[test]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the test picks out the events it asserts on; the other events are not under test"
    )]
    fn normalized_agy_done_only_model_steps_include_checkpoint_usage_without_counting_totals_twice()
    {
        use crate::harness::interpretation::event::{Event, Outcome, Usage};
        let mut stream = stream(Harness::Agy, "");
        for line in [
            r#"{"event":"init","conversation_id":"c","init":{"model":"gemini"}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c","step_index":3,"state":"DONE","step_type":"agent_response","text_delta":"OK","duration_seconds":1.5,"usage":{"input_tokens":100,"output_tokens":12,"thinking_tokens":2,"cache_read_tokens":80}}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c","step_index":4,"state":"DONE","step_type":"checkpoint","usage":{"input_tokens":10,"output_tokens":2}}}"#,
            r#"{"event":"result","result":{"status":"SUCCESS","response":"OK","usage":{"input_tokens":110,"output_tokens":14}}}"#,
        ] {
            stream.decode(line);
        }
        let events = finish(stream).report.unwrap().events;
        let calls: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::ModelCallEnded {
                    id,
                    model,
                    usage,
                    outcome: Outcome::Succeeded,
                    ..
                } => Some((id.as_str(), model.as_deref(), usage.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            [
                (
                    "c:3",
                    Some("gemini"),
                    Usage {
                        input: Some(100),
                        output: Some(12),
                        cache_read: Some(80),
                        ..Usage::default()
                    }
                ),
                (
                    "c:4",
                    Some("gemini"),
                    Usage {
                        input: Some(10),
                        output: Some(2),
                        ..Usage::default()
                    }
                ),
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::ModelCallStarted { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn normalized_agy_denials_limits_and_interruptions_keep_their_evidence() {
        use crate::harness::interpretation::event::{ErrorKind, Event, Outcome};
        let mut stream = stream(Harness::Agy, "");
        stream.decode(r#"{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"tool","tool_name":"write_to_file","tool_info":{"parameters":{"TargetFile":"a"},"error":{"type":"permission_denied","message":"denied"}}}}"#);
        stream.decode(r#"{"event":"result","result":{"status":"ERROR","error":"usage limit exceeded; interrupted"}}"#);
        let events = stream
            .finish(Ok(std::process::ExitStatus::default()))
            .report
            .unwrap()
            .events;
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ToolCallEnded {
                outcome: Outcome::Denied,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Error {
                kind: ErrorKind::UsageLimit,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::WorkEnded {
                outcome: Outcome::Killed | Outcome::Interrupted,
                ..
            }
        )));
    }

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
