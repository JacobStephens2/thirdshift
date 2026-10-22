//! OpenCode step/message IDs identify model calls; tool IDs survive export.
use crate::harness::interpretation::event::{
    Agents, Event, Outcome, Tools, Usage, denied, interrupted,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Events {
    session: String,
    current: String,
    next: usize,
    started: HashSet<String>,
    ended: HashSet<String>,
    tools: Tools,
    agents: Agents,
    text: HashMap<String, Vec<(String, String)>>,
}

impl Events {
    pub(super) fn export(export: &Value, session: &str) -> Vec<Event> {
        let mut decoder = Self::default();
        let mut events = Vec::new();
        for (index, message) in export["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let message_id = message["id"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("message:{index}"));
            let id = format!("{session}:{message_id}");
            if message["type"] == "user" {
                if let Some(text) = message["text"].as_str() {
                    events.push(Event::Message {
                        role: "user".into(),
                        text: text.into(),
                        parent_id: None,
                    });
                }
                continue;
            }
            if message["type"] != "assistant" {
                continue;
            }
            let model = message["model"]["id"].as_str().map(|model| {
                message["model"]["providerID"].as_str().map_or_else(
                    || model.to_string(),
                    |provider| format!("{provider}/{model}"),
                )
            });
            events.push(model_start(&id, model.clone()));
            let mut text = String::new();
            for part in message["content"].as_array().into_iter().flatten() {
                if part["type"] == "tool"
                    && let Some(call_id) = part["id"].as_str()
                {
                    let mut state = part["state"].clone();
                    if state["output"].is_null() {
                        state["output"] = state["content"].clone();
                    }
                    events.extend(decoder.tool(
                        &format!("{session}:tool:{call_id}"),
                        part["name"].as_str().unwrap_or("tool"),
                        &state,
                    ));
                }
                if part["type"] == "text" {
                    text.push_str(part["text"].as_str().unwrap_or(""));
                }
            }
            if !text.is_empty() {
                events.push(Event::Message {
                    role: "assistant".into(),
                    text,
                    parent_id: Some(id.clone()),
                });
            }
            let error = super::error_text(&message["error"]);
            events.push(model_end(
                &id,
                model,
                message,
                error.map_or(Outcome::Succeeded, failure_outcome),
            ));
            if let Some(error) = error {
                events.extend(errors(&id, error, message["error"].clone()));
            }
        }
        if export["info"]["outcome"] == "failed" {
            let message = super::error_text(&export["info"]["error"])
                .unwrap_or("OpenCode's exported outcome is failed");
            events.extend(errors(
                &format!("{session}:outcome"),
                message,
                export["info"]["error"].clone(),
            ));
        }
        events
    }

    pub(super) fn stream(&mut self, event: &Value) -> Vec<Event> {
        if let Some(id) = event["sessionID"].as_str() {
            self.session = id.into();
        }
        let part = &event["part"];
        if let Some(id) = part["messageID"].as_str() {
            self.current = id.into();
        }
        match event["type"].as_str() {
            Some("step_start") => {
                if part["messageID"].is_null() {
                    self.next += 1;
                    self.current = format!("step:{}", self.next);
                }
                let id = format!("{}:{}", self.session, self.current);
                if self.started.insert(id.clone()) {
                    vec![model_start(&id, None)]
                } else {
                    Vec::new()
                }
            }
            Some("step_finish") => {
                let id = format!("{}:{}", self.session, self.current);
                let mut events = Vec::new();
                if self.started.insert(id.clone()) {
                    events.push(model_start(&id, None));
                }
                if self.ended.insert(id.clone()) {
                    events.push(model_end(&id, None, part, Outcome::Succeeded));
                }
                events
            }
            Some("tool_use") => {
                let id = part["callID"]
                    .as_str()
                    .or_else(|| part["id"].as_str())
                    .or_else(|| part["partID"].as_str());
                let Some(id) = id else {
                    return Vec::new();
                };
                self.tool(
                    &format!("{}:tool:{id}", self.session),
                    part["tool"].as_str().unwrap_or("tool"),
                    &part["state"],
                )
            }
            Some("text") => {
                let Some(text) = part["text"].as_str() else {
                    return Vec::new();
                };
                let id = format!("{}:{}", self.session, self.current);
                let parts = self.text.entry(id.clone()).or_default();
                let part_id = part["id"].as_str().unwrap_or("");
                if let Some((_, previous)) = parts.iter_mut().find(|(id, _)| id == part_id) {
                    *previous = text.into();
                } else {
                    parts.push((part_id.into(), text.into()));
                }
                vec![Event::Message {
                    role: "assistant".into(),
                    text: parts.iter().map(|(_, text)| text.as_str()).collect(),
                    parent_id: Some(id),
                }]
            }
            Some("error") => {
                self.next += 1;
                let id = format!("{}:error:{}", self.session, self.next);
                let error = &event["error"];
                let message = format!(
                    "{}: {}",
                    error["type"].as_str().unwrap_or("error"),
                    super::error_text(error).unwrap_or("OpenCode's turn failed")
                );
                let mut events = errors(&id, &message, error.clone());
                if interrupted(&message) {
                    events.extend(self.tools.stop(Outcome::Interrupted));
                    events.extend(self.agents.stop(Outcome::Interrupted));
                }
                events
            }
            _ => Vec::new(),
        }
    }

    fn tool(&mut self, id: &str, name: &str, state: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        let status = state["status"].as_str().unwrap_or("");
        let terminal = matches!(
            status,
            "completed" | "error" | "failed" | "cancelled" | "canceled"
        );
        let failed = status != "completed";
        let result = if failed {
            &state["error"]
        } else {
            &state["output"]
        };
        let outcome = if !failed {
            Outcome::Succeeded
        } else if matches!(status, "cancelled" | "canceled") {
            Outcome::Interrupted
        } else if denied(result) {
            Outcome::Denied
        } else {
            failure_outcome(result.as_str().unwrap_or(status))
        };
        if terminal && outcome == Outcome::Denied {
            events.extend(self.tools.deny(
                id,
                None,
                name.into(),
                state["input"].clone(),
                result.clone(),
            ));
        } else {
            events.extend(
                self.tools
                    .start(id, None, name.into(), state["input"].clone()),
            );
        }
        let agent = format!("{id}:agent");
        if name == "task" {
            events.extend(self.agents.start(
                &agent,
                Some(id.into()),
                state["input"]["subagent_type"].as_str().map(String::from),
            ));
        }
        if terminal && self.ended.insert(id.into()) {
            if outcome != Outcome::Denied {
                events.extend(self.tools.end(id, None, result.clone(), outcome));
            }
            if failed {
                let reason = state["error"].as_str().unwrap_or(status);
                events.push(Event::error(id, reason, state.clone()));
                if outcome == Outcome::Interrupted {
                    events.push(Event::WorkEnded {
                        id: id.into(),
                        outcome,
                    });
                }
            }
            if name == "task" {
                events.extend(self.agents.end(&agent, outcome, result.clone()));
            }
        }
        events
    }
}

fn model_start(id: &str, model: Option<String>) -> Event {
    Event::ModelCallStarted {
        id: id.into(),
        parent_id: None,
        model,
    }
}

fn model_end(id: &str, model: Option<String>, message: &Value, outcome: Outcome) -> Event {
    Event::ModelCallEnded {
        id: id.into(),
        parent_id: None,
        model,
        usage: usage(message),
        cost_usd: message["cost"].as_f64(),
        outcome,
    }
}

fn failure_outcome(message: &str) -> Outcome {
    if interrupted(message) {
        Outcome::Interrupted
    } else {
        Outcome::Failed
    }
}

fn errors(id: &str, message: &str, details: Value) -> Vec<Event> {
    let mut events = vec![Event::error(id, message, details)];
    if interrupted(message) {
        events.push(Event::WorkEnded {
            id: id.into(),
            outcome: Outcome::Interrupted,
        });
    }
    events
}

fn usage(message: &Value) -> Usage {
    let tokens = &message["tokens"];
    let read = tokens["cache"]["read"].as_u64();
    let write = tokens["cache"]["write"].as_u64();
    let reasoning = tokens["reasoning"].as_u64();
    Usage {
        input: tokens["input"]
            .as_u64()
            .map(|input| input + read.unwrap_or(0) + write.unwrap_or(0)),
        output: tokens["output"]
            .as_u64()
            .map(|output| output + reasoning.unwrap_or(0)),
        cache_read: read,
        cache_write: write,
    }
}
