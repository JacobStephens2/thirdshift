//! Normalize step transitions independently of progress's once-per-tool guard.
use crate::harness::interpretation::event::{
    Agents, Event, Outcome, Tools, Usage, UsageScope, denied, interrupted,
};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Events {
    started: HashSet<String>,
    ended: HashSet<String>,
    tools: Tools,
    agents: Agents,
    model: Option<String>,
    conversation: String,
    results: usize,
    last_text: Option<(String, String)>,
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
            if let Some(text) = result["response"].as_str()
                && self
                    .last_text
                    .as_ref()
                    .is_none_or(|(_, previous)| previous != text)
            {
                events.push(Event::Message {
                    id: Some(id.clone()),
                    role: "assistant".into(),
                    text: text.into(),
                    parent_id: None,
                });
            }
            if result["usage"].is_object() {
                events.push(Event::Usage {
                    scope: UsageScope::Session,
                    model: self.model.clone(),
                    usage: usage(&result["usage"]),
                    cost_usd: result["usage"]["cost"].as_f64(),
                });
            }
            if result["status"] != "SUCCESS" {
                let message = result["error"].as_str().unwrap_or("agy's turn failed");
                events.push(Event::error(&id, message, result.clone()));
                if interrupted(message) {
                    events.extend(self.tools.stop(Outcome::Interrupted));
                    events.extend(self.agents.stop(Outcome::Interrupted));
                    events.push(Event::WorkEnded {
                        id,
                        outcome: Outcome::Interrupted,
                    });
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
            let name = step["tool_name"].as_str().unwrap_or("tool").to_string();
            let input = step["tool_info"]["parameters"].clone();
            let error = &step["tool_info"]["error"];
            if step["state"] == "DONE" && denied(error) {
                events.extend(self.tools.deny(&id, None, name, input, error.clone()));
            } else {
                events.extend(self.tools.start(&id, None, name, input));
                if step["state"] == "DONE" {
                    let outcome = if error.is_null() {
                        Outcome::Succeeded
                    } else {
                        Outcome::Failed
                    };
                    events.extend(
                        self.tools.end(
                            &id,
                            None,
                            step["tool_info"]
                                .get("error")
                                .unwrap_or(&step["tool_info"]["output"])
                                .clone(),
                            outcome,
                        ),
                    );
                }
            }
        }
        if step["step_type"] == "agent_response"
            || step["step_type"] == "checkpoint" && step["usage"].is_object()
        {
            if self.started.insert(id.clone()) {
                events.push(Event::ModelCallStarted {
                    id: id.clone(),
                    parent_id: None,
                    model: self.model.clone(),
                });
            }
            if step["state"] == "DONE" && self.ended.insert(id.clone()) {
                events.push(Event::ModelCallEnded {
                    id: id.clone(),
                    parent_id: None,
                    model: self.model.clone(),
                    usage: usage(&step["usage"]),
                    cost_usd: step["usage"]["cost"].as_f64(),
                    outcome: Outcome::Succeeded,
                });
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
            events.extend(self.agents.start(
                &child_id,
                Some(id.clone()),
                child["type_name"].as_str().map(String::from),
            ));
            if step["state"] == "DONE" {
                let outcome = if step["tool_info"]["error"].is_null() {
                    Outcome::Succeeded
                } else {
                    Outcome::Failed
                };
                events.extend(self.agents.end(&child_id, outcome, Value::Null));
            }
        }
        if let Some(text) = step["text_delta"].as_str() {
            let role = match step["step_type"].as_str() {
                Some("user_input") => "user",
                Some("system_message") => "system",
                _ => "assistant",
            };
            if role == "assistant" {
                match &mut self.last_text {
                    Some((step, previous)) if step == &id => previous.push_str(text),
                    previous => *previous = Some((id.clone(), text.into())),
                }
            }
            events.push(Event::Message {
                id: None,
                role: role.into(),
                text: text.into(),
                parent_id: None,
            });
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
    }
}
