//! OpenCode's JSONL progress; final usage is deliberately left to session export.
use crate::harness::interpretation::{Decoder, Ended, Facts, Report, StreamUpdate, TurnOutcome};
use crate::harness::skill_load::SkillLoad;
use crate::progress::{bash, shorten};
use serde_json::Value;

#[derive(Default)]
pub struct OpenCodeProgress {
    session_id: Option<String>,
    started: bool,
    message: Option<String>,
    failure: Option<String>,
    skill_load: SkillLoad,
    events: super::events::Events,
}

impl OpenCodeProgress {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            skill_load: SkillLoad::for_prompt(prompt),
            ..Self::default()
        }
    }
}

impl Decoder for OpenCodeProgress {
    fn condense(&mut self, raw: &str) -> StreamUpdate {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return StreamUpdate::default();
        };
        if let Some(id) = event["sessionID"].as_str().filter(|id| !id.is_empty()) {
            self.session_id = Some(id.into());
        }
        let part = &event["part"];
        let progress = match event["type"].as_str() {
            Some("step_start") if !self.started => {
                self.started = true;
                vec!["session started".into()]
            }
            Some("text") => {
                if let Some(text) = part["text"].as_str() {
                    self.message = Some(text.into());
                }
                Vec::new()
            }
            Some("tool_use") => {
                let tool = part["tool"].as_str().unwrap_or("");
                let state = &part["state"];
                if tool == "skill" {
                    let skill = state["input"]["id"].as_str().unwrap_or("");
                    if state["status"] == "completed" {
                        self.skill_load.observed(skill);
                    }
                    vec![format!("skill {skill}")]
                } else if tool == "bash" {
                    state["input"]["command"]
                        .as_str()
                        .map(|command| vec![bash(command)])
                        .unwrap_or_default()
                } else if tool.is_empty() {
                    Vec::new()
                } else {
                    vec![shorten(tool)]
                }
            }
            Some("step_finish") => vec!["step finished".into()],
            Some("error") => {
                let error = &event["error"];
                let message = super::error_text(error).unwrap_or("OpenCode's turn failed");
                self.failure = Some(if let Some(kind) = error["type"].as_str() {
                    format!("{kind}: {message}")
                } else {
                    message.into()
                });
                vec![format!(
                    "error: {}",
                    shorten(self.failure.as_deref().unwrap())
                )]
            }
            _ => Vec::new(),
        };
        StreamUpdate {
            events: self.events.stream(&event),
            progress,
            evidence: Vec::new(),
        }
    }
    fn complete(self: Box<Self>) -> Facts {
        let outcome = TurnOutcome::from_failed(self.failure.is_some());
        Facts {
            report: Report {
                warnings: self.skill_load.warnings(),
                summary: None,
                ..Report::default()
            },
            ended: Ended {
                session_id: self.session_id,
                // Tool cancellation alone is not killed-work evidence.
                killed: Vec::new(),
                final_message: self.message,
            },
            outcome,
            diagnostic: self.failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };

    #[test]
    fn normalized_opencode_steps_and_completed_only_tools_keep_reported_times_and_usage() {
        use crate::harness::interpretation::event::{Event, Outcome, Usage};
        let mut stream = stream(Harness::OpenCode, "");
        let mut live = Vec::new();
        for line in include_str!("../../../tests/fixtures/normalized-opencode.jsonl").lines() {
            live.extend(stream.decode(line).events);
        }
        assert_eq!(live.len(), 6);
        assert!(matches!(live[0], Event::ModelCallStarted { .. }));
        assert!(
            matches!(&live[1], Event::ToolCallStarted { tool, input, .. } if tool == "bash" && input["command"] == "cargo test")
        );
        assert_eq!(
            live[3],
            Event::ModelCallEnded {
                id: "s:m1".into(),
                parent_id: None,
                model: None,
                usage: Usage {
                    input: Some(7515),
                    output: Some(23),
                    cache_read: Some(7424),
                    cache_write: Some(0),
                },
                cost_usd: Some(0.02),
                outcome: Outcome::Succeeded,
            }
        );
        assert!(matches!(&live[0], Event::ModelCallStarted { id, .. } if id == "s:m1"));
        assert!(finish(stream).report.unwrap().summary.is_none());
    }

    #[test]
    fn normalized_opencode_errors_denials_and_cancelled_tools_survive_failure() {
        use crate::harness::interpretation::event::{ErrorKind, Event, Outcome};
        let mut stream = stream(Harness::OpenCode, "");
        for line in [
            r#"{"type":"tool_use","sessionID":"s","part":{"id":"t","tool":"bash","state":{"status":"error","input":{"command":"cargo test"},"error":"permission denied"}}}"#,
            r#"{"type":"tool_use","sessionID":"s","part":{"id":"b","tool":"bash","state":{"status":"cancelled","error":"session ended"}}}"#,
            r#"{"type":"error","sessionID":"s","error":{"type":"usage_limit","message":"quota exhausted"}}"#,
        ] {
            stream.decode(line);
        }
        let completion = finish(stream);
        assert!(completion.outcome.is_err());
        let events = completion.report.unwrap().events;
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ToolCallEnded {
                id,
                outcome: Outcome::Denied,
                ..
            } if id == "s:tool:t"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Error {
                kind: ErrorKind::UsageLimit,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(event, Event::WorkEnded {
                    id,
                    outcome: Outcome::Killed | Outcome::Interrupted,
                } if id == "s:tool:b")));
    }

    #[test]
    fn a_dropped_last_step_keeps_the_last_text_without_trusting_stream_usage() {
        let mut stream = stream(Harness::OpenCode, "/thirdshift-implement issue-url");
        for line in [
            r#"{"type":"step_start","sessionID":"s1","part":{}}"#,
            r#"{"type":"tool_use","sessionID":"s1","part":{"tool":"skill","state":{"status":"completed","input":{"id":"thirdshift-implement"}}}}"#,
            r#"{"type":"text","sessionID":"s1","part":{"text":"earlier reply"}}"#,
            r#"{"type":"step_finish","sessionID":"s1","part":{"tokens":{"input":1200,"output":300}}}"#,
            r#"{"type":"text","sessionID":"s1","part":{"text":"last reply"}}"#,
            "not JSON",
        ] {
            stream.condense(line);
        }
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("last reply")
        );
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("s1")
        );
        assert!(stream.report.as_ref().unwrap().warnings.clone().is_empty());
        assert!(stream.report.as_ref().unwrap().summary.clone().is_none());
    }

    #[test]
    fn a_wrong_or_failed_skill_load_still_warns() {
        let mut stream = stream(Harness::OpenCode, "/thirdshift-implement issue-url");
        for line in [
            r#"{"type":"tool_use","part":{"tool":"skill","state":{"status":"completed","input":{"id":"thirdshift-tdd"}}}}"#,
            r#"{"type":"tool_use","part":{"tool":"skill","state":{"status":"error","input":{"id":"thirdshift-implement"}}}}"#,
        ] {
            stream.condense(line);
        }
        let stream = finish(stream);
        assert_eq!(
            stream.report.as_ref().unwrap().warnings.clone(),
            ["warning: the session never loaded thirdshift-implement with its skill tool"]
        );
        assert!(
            stream
                .outcome
                .as_ref()
                .unwrap()
                .session_id
                .as_deref()
                .is_none()
        );
    }

    #[test]
    fn a_recorded_no_route_error_fails_and_names_the_model_or_effort_problem() {
        let mut stream = stream(Harness::OpenCode, "");
        stream.condense(r#"{"type":"error","sessionID":"s1","error":{"type":"provider.no-route","message":"Model unavailable: thirdshift-invalid-provider/invalid"}}"#);
        let stream = finish(stream);
        assert!(stream.outcome.is_err());
        assert_eq!(
            stream.outcome.as_ref().unwrap_err().to_string(),
            "opencode's turn failed: provider.no-route: Model unavailable: thirdshift-invalid-provider/invalid"
        );
    }
}
