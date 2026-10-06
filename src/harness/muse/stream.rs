//! Muse's JSONL event envelopes. Usage is read from its session log after exit.

use crate::harness::interpretation::{Decoder, Ended, Facts, Report, TurnOutcome};
use crate::harness::skill_load::SkillLoad;
use crate::progress::{bash, shorten};
use serde_json::Value;

#[derive(Default)]
pub struct MuseProgress {
    session_id: Option<String>,
    started: bool,
    final_message: Option<String>,
    failure: Option<String>,
    skill_load: SkillLoad,
    tasks: Vec<(String, String)>,
    killed: Vec<String>,
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
    fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        if event["stream"]["kind"] == "session"
            && let Some(id) = event["stream"]["id"].as_str()
        {
            self.session_id = Some(id.to_string());
        }
        let payload = &event["payload"];
        match event["payload_type"].as_str() {
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
        }
    }
    fn complete(self: Box<Self>) -> Facts {
        let outcome = TurnOutcome::from_failed(self.failure.is_some());
        Facts {
            report: Report {
                warnings: self.skill_load.warnings(),
                summary: None,
            },
            ended: Ended {
                session_id: self.session_id,
                killed: self.killed,
                final_message: self.final_message,
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
    use serde_json::json;

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
