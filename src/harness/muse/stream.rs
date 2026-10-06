//! Muse's JSONL event envelopes. Usage is read from its session log after exit.

use crate::progress::{Stream, bash, shorten};
use serde_json::Value;

#[derive(Default)]
pub struct MuseProgress {
    session_id: Option<String>,
    started: bool,
    final_message: Option<String>,
    failure: Option<String>,
    expected_skill: Option<String>,
    skill_loaded: bool,
    tasks: Vec<(String, String)>,
    killed: Vec<String>,
}

impl MuseProgress {
    pub fn for_prompt(prompt: &str) -> Self {
        Self {
            expected_skill: prompt
                .strip_prefix('/')
                .and_then(|rest| rest.split_whitespace().next())
                .filter(|name| name.starts_with("thirdshift-"))
                .map(String::from),
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
            if facts["outcome"] == "success"
                && header.contains("status=\"ok\"")
                && self.expected_skill.as_deref() == Some(skill)
            {
                self.skill_loaded = true;
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

impl Stream for MuseProgress {
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
    fn summary(&self) -> Option<String> {
        None
    }
    fn final_message(&self) -> Option<&str> {
        self.final_message.as_deref()
    }
    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    fn killed_background_work(&self) -> Vec<&str> {
        if self.failed() {
            Vec::new()
        } else {
            self.killed.iter().map(String::as_str).collect()
        }
    }
    fn failed(&self) -> bool {
        self.failure.is_some()
    }
    fn error(&self) -> Option<&str> {
        self.failure.as_deref()
    }
    fn warnings(&self) -> Vec<String> {
        self.expected_skill
            .as_ref()
            .filter(|_| !self.skill_loaded)
            .map(|skill| {
                vec![format!(
                    "warning: the session never loaded {skill} with its skill tool"
                )]
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_successful_read_skill_result_loads_only_the_named_skill() {
        let mut stream = MuseProgress::for_prompt(
            "/thirdshift-implement https://github.com/acme/widgets/issues/7",
        );
        let result = |name: &str| {
            json!({"stream":{"kind":"session","id":"s1"},"payload_type":"tool.result", "payload":{
            "correlation_facts":{"tool_name":"read_skill","outcome":"success"},
            "text":format!("<read-skill-result name=\"{name}\" status=\"ok\">\n<metadata>path: /skill/SKILL.md</metadata>\n</read-skill-result>")
        }}).to_string()
        };
        stream.condense(&result("thirdshift-tdd"));
        assert_eq!(stream.warnings().len(), 1);
        assert_eq!(
            stream.condense(&result("thirdshift-implement")),
            ["skill thirdshift-implement"]
        );
        assert!(stream.warnings().is_empty());
        assert_eq!(stream.session_id(), Some("s1"));
    }
}
