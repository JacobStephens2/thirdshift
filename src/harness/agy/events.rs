//! Normalize step transitions independently of progress's once-per-tool guard.
use crate::harness::interpretation::event::{Event, Kind, Usage, denied, interrupted};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Events {
    started: HashSet<String>,
    ended: HashSet<String>,
    model: Option<String>,
    conversation: String,
    messages: usize,
    results: usize,
}

impl Events {
    pub(super) fn decode(&mut self, event: &Value) -> Vec<Event> {
        if event["event"] == "init" {
            self.model = event["init"]["model"].as_str().map(String::from);
            self.conversation = event["conversation_id"].as_str().unwrap_or("").into();
        }
        if event["event"] == "result" {
            self.results += 1;
            let id = format!("{}:result:{}", self.conversation, self.results);
            let result = &event["result"];
            let mut events = Vec::new();
            if let Some(text) = result["response"].as_str() {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::Message {
                        role: "assistant".into(),
                        text: text.into(),
                    },
                ));
            }
            if result["usage"].is_object() {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::UsageSummary(usage(&result["usage"])),
                ));
            }
            if result["status"] != "SUCCESS" {
                let message = result["error"].as_str().unwrap_or("agy's turn failed");
                events.push(Event::error(&id, None, message));
                if interrupted(message) {
                    events.push(Event::new(
                        id,
                        None,
                        Kind::Killed {
                            description: message.into(),
                        },
                    ));
                }
            }
            return events;
        }
        if event["event"] != "step_update" {
            return Vec::new();
        }
        let step = &event["step_update"];
        let Some(index) = step["step_index"].as_u64() else {
            return Vec::new();
        };
        let id = format!(
            "{}:{index}",
            step["conversation_id"]
                .as_str()
                .unwrap_or(&self.conversation)
        );
        let mut events = Vec::new();
        if step["step_type"] == "tool" {
            if self.started.insert(id.clone()) {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::ToolStarted {
                        name: step["tool_name"].as_str().unwrap_or("tool").into(),
                        input: step["tool_info"]["parameters"].clone(),
                    },
                ));
            }
            if step["state"] == "DONE" && self.ended.insert(id.clone()) {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::ToolEnded {
                        result: step["tool_info"]
                            .get("error")
                            .unwrap_or(&step["tool_info"]["output"])
                            .clone(),
                        failed: !step["tool_info"]["error"].is_null(),
                        denied: denied(&step["tool_info"]["error"]),
                    },
                ));
            }
        }
        if step["step_type"] == "agent_response"
            || step["step_type"] == "checkpoint" && step["usage"].is_object()
        {
            if self.started.insert(id.clone()) {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::ModelStarted {
                        model: self.model.clone(),
                    },
                ));
            }
            if step["state"] == "DONE" && self.ended.insert(id.clone()) {
                events.push(Event::new(
                    &id,
                    None,
                    Kind::ModelEnded {
                        model: self.model.clone(),
                        usage: usage(&step["usage"]),
                        failed: false,
                    },
                ));
            }
        }
        for child in step["subagent_info"]["subagents"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(child_id) = child["conversation_id"].as_str() else {
                continue;
            };
            let child_id = format!("{id}:agent:{child_id}");
            if self.started.insert(child_id.clone()) {
                events.push(Event::new(
                    &child_id,
                    None,
                    Kind::AgentStarted {
                        name: child["type_name"].as_str().unwrap_or("subagent").into(),
                    },
                ));
            }
            if step["state"] == "DONE" && self.ended.insert(child_id.clone()) {
                events.push(Event::new(
                    child_id,
                    None,
                    Kind::AgentEnded {
                        failed: !step["tool_info"]["error"].is_null(),
                    },
                ));
            }
        }
        if let Some(text) = step["text_delta"].as_str() {
            self.messages += 1;
            let role = match step["step_type"].as_str() {
                Some("user_input") => "user",
                Some("system_message") => "system",
                _ => "assistant",
            };
            events.push(Event::new(
                format!("{id}:text:{}", self.messages),
                None,
                Kind::Message {
                    role: role.into(),
                    text: text.into(),
                },
            ));
        }
        if let Some(duration) = step["duration_seconds"]
            .as_f64()
            .filter(|duration| duration.is_finite() && *duration >= 0.0)
        {
            for event in &mut events {
                if matches!(event.kind, Kind::ModelEnded { .. } | Kind::ToolEnded { .. }) {
                    event.duration_nanos = Some((duration * 1_000_000_000.0) as u64);
                }
            }
        }
        events
    }
}

fn usage(usage: &Value) -> Usage {
    Usage {
        input: usage["input_tokens"].as_u64(),
        output: usage["output_tokens"].as_u64(),
        cache_read: usage["cache_read_tokens"].as_u64(),
        cache_write: usage["cache_write_tokens"].as_u64(),
        reasoning: usage["thinking_tokens"].as_u64(),
        cost: usage["cost"].as_f64(),
    }
}
