//! Muse's JSONL event envelopes. Usage is read from its session log after exit.

use crate::harness::interpretation::{Decoder, Ended, Facts, Report, StreamUpdate, TurnOutcome};
use crate::harness::skill_load::SkillLoad;
use crate::progress::{bash, shorten};
use serde_json::Value;

#[derive(Default)]
pub struct MuseProgress {
    session_id: Option<String>,
    started: bool,
    final_message: Option<String>,
    completed: bool,
    failure: Option<String>,
    skill_load: SkillLoad,
    tasks: Vec<(String, String)>,
    killed: Vec<String>,
    events: super::events::Events,
}

impl MuseProgress {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            skill_load: SkillLoad::for_prompt(prompt),
            ..Self::default()
        }
    }

    fn tool(&mut self, payload: &Value) -> Vec<String> {
        let facts = &payload["correlation_facts"];
        let name = facts["tool_name"].as_str().unwrap_or("");
        let text = payload["text"].as_str().unwrap_or("");
        if name == "read_skill" {
            let header = text.lines().next().unwrap_or("");
            let skill = header
                .strip_prefix("<read-skill-result name=\"")
                .and_then(|rest| rest.split_once('"'))
                .map(|(name, _)| name)
                .unwrap_or("");
            if facts["outcome"] == "success" && header.contains("status=\"ok\"") {
                self.skill_load.observed(skill);
            }
            return vec![format!("skill {skill}")];
        }
        if name == "bash" {
            let result: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            return result["command"]
                .as_str()
                .map(|command| vec![bash(command)])
                .unwrap_or_default();
        }
        if name.is_empty() {
            Vec::new()
        } else {
            vec![shorten(name)]
        }
    }
}

impl Decoder for MuseProgress {
    fn condense(&mut self, raw: &str) -> StreamUpdate {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return StreamUpdate::default();
        };
        if event["stream"]["kind"] == "session"
            && let Some(id) = event["stream"]["id"].as_str()
        {
            self.session_id = Some(id.to_string());
        }
        let payload = &event["payload"];
        let progress = match event["payload_type"].as_str() {
            Some("run.lifecycle.started") if !self.started => {
                self.started = true;
                vec!["session started".into()]
            }
            Some("task.lifecycle.proposed")
                if payload["event"]["task_kind"]
                    .as_str()
                    .is_some_and(|kind| kind.starts_with("model.")) =>
            {
                self.final_message = None;
                Vec::new()
            }
            Some("task.lifecycle.proposed") => {
                let event = &payload["event"];
                if let (Some(id), Some(kind)) =
                    (event["task_id"].as_str(), event["task_kind"].as_str())
                    && kind.starts_with("tool.")
                {
                    self.tasks.push((id.into(), kind.into()));
                }
                Vec::new()
            }
            Some("task.lifecycle.cancelled") => {
                let event = &payload["event"];
                if event["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("session ended"))
                    && let Some(id) = event["task_id"].as_str()
                    && let Some((_, description)) = self.tasks.iter().find(|(task, _)| task == id)
                {
                    self.killed.push(description.clone());
                }
                Vec::new()
            }
            Some("run.output.delta") => {
                if let Some(text) = payload["text"].as_str() {
                    self.final_message.get_or_insert_default().push_str(text);
                }
                Vec::new()
            }
            Some("run.terminal.completed") => {
                self.completed = true;
                if self.final_message.is_none() {
                    self.final_message = payload["text"].as_str().map(String::from);
                }
                Vec::new()
            }
            Some("run.terminal.failed") => {
                self.failure = Some(
                    payload["reason"]
                        .as_str()
                        .unwrap_or("Muse's turn failed")
                        .to_string(),
                );
                vec![format!(
                    "error: {}",
                    shorten(self.failure.as_deref().unwrap())
                )]
            }
            Some("tool.result") => self.tool(payload),
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
                killed: self.killed,
                final_message: self.final_message,
                completed: self.completed,
            },
            outcome,
            diagnostic: self.failure,
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn review_muse_successful_terminal_fallback_is_normalized() {
        use crate::harness::interpretation::{Interpretation, event::Event};
        let mut interpretation =
            Interpretation::new("muse", Box::new(super::MuseProgress::default()));
        interpretation.decode(
        r#"{"stream":{"kind":"session","id":"s"},"payload_type":"run.terminal.completed","payload":{"command_id":"r","text":"Terminal fallback."}}"#,
    );
        let completion = finish(interpretation);
        assert_eq!(
            completion.outcome.unwrap().final_message.as_deref(),
            Some("Terminal fallback.")
        );
        let events = completion.report.unwrap().events;
        assert!(events.iter().any(|event| matches!(event,
        Event::Message { role, text, .. } if role == "assistant" && text == "Terminal fallback."
    )), "available terminal text is missing: {events:?}");
    }
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };
    use serde_json::json;

    #[test]
    fn normalized_muse_lifecycle_edges_are_ordered_and_tool_results_enrich_the_same_call() {
        use crate::harness::interpretation::event::{Event, Outcome};
        let mut stream = stream(Harness::Muse, "");
        for line in include_str!("../../../tests/fixtures/normalized-muse.jsonl").lines() {
            stream.decode(line);
        }
        let events = finish(stream).report.unwrap().events;
        assert_eq!(events.len(), 4);
        assert_eq!(
            events
                .iter()
                .filter_map(
                    #[expect(
                        clippy::wildcard_enum_match_arm,
                        reason = "the test asserts on call ids; other events are not under test"
                    )]
                    |event| match event {
                        Event::ModelCallStarted { id, .. }
                        | Event::ModelCallEnded { id, .. }
                        | Event::ToolCallStarted { id, .. }
                        | Event::ToolCallEnded { id, .. } => Some(id.as_str()),
                        _ => None,
                    },
                )
                .collect::<Vec<_>>(),
            ["r:m1", "r:m1", "r:t1", "r:t1"]
        );
        assert!(
            matches!(&events[0], Event::ModelCallStarted { model: Some(model), .. } if model == "muse-spark-1.3")
        );
        assert!(matches!(
            events[1],
            Event::ModelCallEnded {
                outcome: Outcome::Succeeded,
                ..
            }
        ));
        assert!(
            matches!(&events[2], Event::ToolCallStarted { tool, input, .. } if tool == "bash" && input["command"] == "cargo test")
        );
        assert!(
            matches!(&events[3], Event::ToolCallEnded { result, outcome: Outcome::Succeeded, .. } if result.as_str().unwrap().contains("done"))
        );
    }

    #[test]
    fn normalized_muse_denials_limits_killed_tasks_and_agents_keep_their_outcomes() {
        use crate::harness::interpretation::event::{ErrorKind, Event, Outcome};
        let mut stream = stream(Harness::Muse, "");
        for (kind, event) in [
            (
                "proposed",
                json!({"task_id":"m","task_kind":"model.meta.response"}),
            ),
            ("started", json!({"task_id":"m"})),
            ("failed", json!({"task_id":"m","reason":"rate limit 429"})),
            ("proposed", json!({"task_id":"t","task_kind":"tool.bash"})),
            ("started", json!({"task_id":"t"})),
            ("cancelled", json!({"task_id":"t","reason":"session ended"})),
            ("proposed", json!({"task_id":"d","task_kind":"tool.write"})),
            (
                "failed",
                json!({"task_id":"d","reason":"permission denied"}),
            ),
            (
                "proposed",
                json!({"task_id":"a","task_kind":"reminder.agent.verify"}),
            ),
            ("started", json!({"task_id":"a"})),
            ("completed", json!({"task_id":"a"})),
        ] {
            stream.decode(&json!({"stream":{"kind":"session","id":"s"},"payload_type":format!("task.lifecycle.{kind}"),"payload":{"event":event}}).to_string());
        }
        stream.decode(
            r#"{"payload_type":"run.terminal.failed","payload":{"reason":"usage limit exceeded"}}"#,
        );
        let events = finish(stream).report.unwrap().events;
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Error {
                kind: ErrorKind::RateLimit,
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
            Event::ToolCallEnded {
                id,
                outcome: Outcome::Denied,
                ..
            } if id == "s:d"
        )));
        assert!(events.iter().any(|event| matches!(event, Event::WorkEnded {
                    id,
                    outcome: Outcome::Killed | Outcome::Interrupted,
                } if id == "s:t")));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::SubagentStarted { .. }))
        );
        assert!(events.iter().any(|event| matches!(
            event,
            Event::SubagentEnded {
                outcome: Outcome::Succeeded,
                ..
            }
        )));
    }

    #[test]
    fn the_stream_fallback_keeps_the_last_reply_instead_of_muses_joined_terminal_text() {
        // Muse's recorded two-reply sequence, with irrelevant envelope fields removed.
        let lines = [
            r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.output.delta","payload":{"text":"MANGO"}}"#,
            r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"task.lifecycle.proposed","payload":{"event":{"task_kind":"model.meta.response"}}}"#,
            r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.output.delta","payload":{"text":"OK MANGO"}}"#,
            r#"{"stream":{"kind":"session","id":"s1"},"payload_type":"run.terminal.completed","payload":{"text":"MANGOOK MANGO"}}"#,
        ];
        let mut stream = stream(Harness::Muse, "");
        for line in lines {
            stream.condense(line);
        }
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("OK MANGO")
        );
        assert_eq!(stream.report.as_ref().unwrap().summary.clone(), None);
    }

    #[test]
    fn a_successful_read_skill_result_loads_only_the_named_skill() {
        let mut stream = stream(
            Harness::Muse,
            "/thirdshift-implement https://github.com/acme/widgets/issues/7",
        );
        let result = |name: &str| {
            json!({"stream":{"kind":"session","id":"s1"},"payload_type":"tool.result", "payload":{
            "correlation_facts":{"tool_name":"read_skill","outcome":"success"},
            "text":format!("<read-skill-result name=\"{name}\" status=\"ok\">\n<metadata>path: /skill/SKILL.md</metadata>\n</read-skill-result>")
        }}).to_string()
        };
        stream.condense(&result("thirdshift-tdd"));

        assert_eq!(
            stream.condense(&result("thirdshift-implement")),
            ["skill thirdshift-implement"]
        );
        let stream = finish(stream);
        assert!(stream.report.as_ref().unwrap().warnings.clone().is_empty());
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("s1")
        );
    }
}
