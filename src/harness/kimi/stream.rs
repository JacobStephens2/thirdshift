//! Kimi Code's `-p --output-format stream-json` lines: OpenAI-shaped
//! assistant and tool messages, plus `meta` lines for its version, retries
//! and, after a completed turn only, the id to resume. Errors before or
//! outside a turn go to stderr and the exit status, except an occupied
//! session's, which is an `error` line. The stream carries no usage; the
//! session's `wire.jsonl` does.

use std::collections::HashMap;

use serde_json::Value;

use crate::harness::interpretation::event::{Event, Outcome, Tools, interrupted};
use crate::harness::interpretation::{Decoder, Ended, Facts, Report, StreamUpdate, TurnOutcome};
use crate::harness::skill_load::SkillLoad;
use crate::progress::{bash, shorten};

#[derive(Default)]
pub struct KimiProgress {
    session_id: Option<String>,
    started: bool,
    message: Option<String>,
    completed: bool,
    failure: Option<String>,
    skill_load: SkillLoad,
    /// Each tool call's name and arguments by id, until its result arrives.
    calls: HashMap<String, (String, Value)>,
    tools: Tools,
    messages: usize,
}

impl KimiProgress {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            skill_load: SkillLoad::for_prompt(prompt),
            ..Self::default()
        }
    }

    fn assistant(&mut self, event: &Value) -> (Vec<String>, Vec<Event>) {
        let mut progress = Vec::new();
        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            progress.push("session started".into());
        }
        if let Some(text) = event["content"].as_str().filter(|text| !text.is_empty()) {
            self.message = Some(text.into());
            self.messages += 1;
            events.push(Event::Message {
                id: Some(format!("message:{}", self.messages)),
                role: "assistant".into(),
                text: text.into(),
                parent_id: None,
            });
        }
        for call in event["tool_calls"].as_array().into_iter().flatten() {
            let Some(id) = call["id"].as_str() else {
                continue;
            };
            let name = call["function"]["name"].as_str().unwrap_or("tool");
            let arguments = &call["function"]["arguments"];
            let input = arguments
                .as_str()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_else(|| arguments.clone());
            progress.extend(match name {
                "Bash" => input["command"].as_str().map(bash),
                "Skill" => input["skill"]
                    .as_str()
                    .map(|skill| format!("skill {skill}")),
                _ => Some(shorten(name)),
            });
            events.extend(self.tools.start(id, None, name.into(), input.clone()));
            self.calls.insert(id.into(), (name.into(), input));
        }
        (progress, events)
    }

    fn tool(&mut self, event: &Value) -> Vec<Event> {
        let Some(id) = event["tool_call_id"].as_str() else {
            return Vec::new();
        };
        let content = event["content"].clone();
        let text = content.as_str().unwrap_or("");
        if let Some((name, input)) = self.calls.remove(id)
            && name == "Skill"
            && text.starts_with("Skill \"")
            && text.contains(" loaded")
            && let Some(skill) = input["skill"].as_str()
        {
            self.skill_load.observed(skill);
        }
        let outcome = if interrupted(text) {
            Outcome::Interrupted
        } else {
            Outcome::Succeeded
        };
        self.tools
            .end(id, None, content, outcome)
            .into_iter()
            .collect()
    }
}

impl Decoder for KimiProgress {
    fn condense(&mut self, raw: &str) -> StreamUpdate {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return StreamUpdate::default();
        };
        let (progress, events) = match (event["role"].as_str(), event["type"].as_str()) {
            (Some("assistant"), _) => self.assistant(&event),
            (Some("tool"), _) => (Vec::new(), self.tool(&event)),
            (Some("meta"), Some("session.resume_hint")) => {
                if let Some(id) = event["session_id"].as_str().filter(|id| !id.is_empty()) {
                    self.session_id = Some(id.into());
                }
                self.completed = true;
                (Vec::new(), Vec::new())
            }
            (Some("meta"), Some("turn.step.retrying")) => {
                let error = event["error_message"].as_str().unwrap_or("request failed");
                (
                    vec![format!("retrying: {}", shorten(error))],
                    vec![Event::error("retry", error, event.clone())],
                )
            }
            (None, Some("error")) => {
                let message = event["message"]
                    .as_str()
                    .unwrap_or("Kimi Code's turn failed");
                self.failure = Some(match event["code"].as_str() {
                    Some(code) => format!("{code}: {message}"),
                    None => message.into(),
                });
                (
                    vec![format!("error: {}", shorten(message))],
                    vec![Event::error("session", message, event.clone())],
                )
            }
            _ => (Vec::new(), Vec::new()),
        };
        StreamUpdate {
            events,
            progress,
            evidence: Vec::new(),
        }
    }

    fn complete(self: Box<Self>) -> Facts {
        Facts {
            report: Report {
                warnings: self.skill_load.warnings(),
                ..Report::default()
            },
            ended: Ended {
                session_id: self.session_id,
                // Print mode waits for or steers its own background work.
                killed: Vec::new(),
                final_message: self.message,
                completed: self.completed,
            },
            outcome: TurnOutcome::from_failed(self.failure.is_some()),
            diagnostic: self.failure,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::harness::Harness;
    use crate::harness::interpretation::event::Event;
    use crate::harness::interpretation_tests::{finish, stream};

    const PROMPT: &str = "/thirdshift-implement issue-url";

    #[test]
    fn tools_skills_and_the_last_reply_come_from_the_stream_and_the_hint_names_the_session() {
        let mut stream = stream(Harness::Kimi, PROMPT);
        let mut progress = Vec::new();
        let mut events = Vec::new();
        for line in include_str!("../../../tests/fixtures/normalized-kimi.jsonl").lines() {
            let decoded = stream.decode(line);
            progress.extend(decoded.progress);
            events.extend(decoded.events);
        }
        assert_eq!(
            progress,
            [
                "session started",
                "skill thirdshift-implement",
                "$ cargo test"
            ]
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Event::ToolCallEnded { tool, .. } if tool == "Bash"
        )));
        let finished = finish(stream);
        assert!(finished.report.unwrap().warnings.is_empty());
        let ended = finished.outcome.unwrap();
        assert_eq!(ended.session_id.as_deref(), Some("ses_7f3c2a"));
        assert_eq!(
            ended.final_message.as_deref(),
            Some("Opened the pull request.")
        );
        assert!(ended.completed);
    }

    #[test]
    fn without_the_resume_hint_the_turn_did_not_complete_and_has_no_id() {
        let mut stream = stream(Harness::Kimi, PROMPT);
        stream.condense(r#"{"role":"assistant","content":"partial"}"#);
        let finished = finish(stream);
        assert_eq!(
            finished.report.unwrap().warnings,
            ["warning: the session never loaded thirdshift-implement with its skill tool"]
        );
        let ended = finished.outcome.unwrap();
        assert!(!ended.completed);
        assert_eq!(ended.session_id, None);
    }

    #[test]
    fn a_failed_skill_load_still_warns() {
        let mut stream = stream(Harness::Kimi, PROMPT);
        for line in [
            r#"{"role":"assistant","tool_calls":[{"type":"function","id":"c1","function":{"name":"Skill","arguments":"{\"skill\":\"thirdshift-implement\"}"}}]}"#,
            r#"{"role":"tool","tool_call_id":"c1","content":"Skill not found: thirdshift-implement"}"#,
        ] {
            stream.condense(line);
        }
        assert_eq!(finish(stream).report.unwrap().warnings.len(), 1);
    }

    #[test]
    fn an_error_line_fails_the_turn_with_its_code_and_message() {
        let mut stream = stream(Harness::Kimi, "");
        stream.condense(r#"{"type":"error","code":"session_occupied","message":"Session is in use by another client"}"#);
        assert_eq!(
            finish(stream).outcome.unwrap_err().to_string(),
            "kimi's turn failed: session_occupied: Session is in use by another client"
        );
    }
}
