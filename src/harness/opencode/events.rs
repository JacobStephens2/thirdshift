//! OpenCode step/message IDs identify model calls; tool IDs survive export.
use crate::harness::interpretation::event::{Event, Kind, Usage, denied, interrupted, nanos};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct Events {
    session: String,
    current: String,
    next: usize,
    started: HashSet<String>,
    ended: HashSet<String>,
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
            let start = nanos(&message["time"]["created"], 1_000_000);
            let end = nanos(&message["time"]["completed"], 1_000_000);
            if message["type"] == "user" {
                if let Some(text) = message["text"].as_str() {
                    events.push(Event::new(
                        id,
                        start,
                        Kind::Message {
                            role: "user".into(),
                            text: text.into(),
                        },
                    ));
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
            events.push(Event::new(
                &id,
                start,
                Kind::ModelStarted {
                    model: model.clone(),
                },
            ));
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
                        nanos(&part["time"]["created"], 1_000_000),
                        nanos(&part["time"]["completed"], 1_000_000),
                    ));
                }
                if part["type"] == "text" {
                    text.push_str(part["text"].as_str().unwrap_or(""));
                }
            }
            if !text.is_empty() {
                events.push(Event::new(
                    format!("{id}:message"),
                    end,
                    Kind::Message {
                        role: "assistant".into(),
                        text,
                    },
                ));
            }
            let mut event = Event::new(
                &id,
                end,
                Kind::ModelEnded {
                    model,
                    usage: usage(message),
                    failed: !message["error"].is_null(),
                },
            );
            event.duration_nanos = start
                .zip(end)
                .and_then(|(start, end)| end.checked_sub(start));
            events.push(event);
            if let Some(message) = super::error_text(&message["error"]) {
                events.push(Event::error(&id, end, message));
                if interrupted(message) {
                    events.push(Event::new(
                        &id,
                        end,
                        Kind::Killed {
                            description: message.into(),
                        },
                    ));
                }
            }
        }
        if export["info"]["outcome"] == "failed" {
            let message = super::error_text(&export["info"]["error"])
                .unwrap_or("OpenCode's exported outcome is failed");
            events.push(Event::error(format!("{session}:outcome"), None, message));
            if interrupted(message) {
                events.push(Event::new(
                    format!("{session}:outcome"),
                    None,
                    Kind::Killed {
                        description: message.into(),
                    },
                ));
            }
        }
        events
    }

    pub(super) fn stream(&mut self, event: &Value) -> Vec<Event> {
        if let Some(id) = event["sessionID"].as_str() {
            self.session = id.into();
        }
        let part = &event["part"];
        let time = nanos(&event["timestamp"], 1_000_000);
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
                    vec![Event::new(id, time, Kind::ModelStarted { model: None })]
                } else {
                    Vec::new()
                }
            }
            Some("step_finish") => {
                let id = format!("{}:{}", self.session, self.current);
                let mut events = Vec::new();
                if self.started.insert(id.clone()) {
                    events.push(Event::new(&id, None, Kind::ModelStarted { model: None }));
                }
                if self.ended.insert(id.clone()) {
                    events.push(Event::new(
                        id,
                        time,
                        Kind::ModelEnded {
                            model: None,
                            usage: usage(part),
                            failed: false,
                        },
                    ));
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
                    nanos(&part["state"]["time"]["start"], 1_000_000),
                    nanos(&part["state"]["time"]["end"], 1_000_000).or(time),
                )
            }
            Some("text") => {
                let Some(text) = part["text"].as_str() else {
                    return Vec::new();
                };
                let id = format!("{}:{}:message", self.session, self.current);
                let parts = self.text.entry(id.clone()).or_default();
                let part_id = part["id"].as_str().unwrap_or("");
                if let Some((_, previous)) = parts.iter_mut().find(|(id, _)| id == part_id) {
                    *previous = text.into();
                } else {
                    parts.push((part_id.into(), text.into()));
                }
                vec![Event::new(
                    id,
                    time,
                    Kind::Message {
                        role: "assistant".into(),
                        text: parts.iter().map(|(_, text)| text.as_str()).collect(),
                    },
                )]
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
                let mut events = vec![Event::error(&id, time, &message)];
                if interrupted(&message) {
                    events.push(Event::new(
                        id,
                        time,
                        Kind::Killed {
                            description: message,
                        },
                    ));
                }
                events
            }
            _ => Vec::new(),
        }
    }

    fn tool(
        &mut self,
        id: &str,
        name: &str,
        state: &Value,
        start: Option<u64>,
        end: Option<u64>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        if self.started.insert(id.into()) {
            events.push(Event::new(
                id,
                start,
                Kind::ToolStarted {
                    name: name.into(),
                    input: state["input"].clone(),
                },
            ));
        }
        if name == "task" && self.started.insert(format!("{id}:agent")) {
            events.push(Event::new(
                format!("{id}:agent"),
                start,
                Kind::AgentStarted {
                    name: state["input"]["subagent_type"]
                        .as_str()
                        .unwrap_or("task")
                        .into(),
                },
            ));
        }
        let status = state["status"].as_str().unwrap_or("");
        if matches!(
            status,
            "completed" | "error" | "failed" | "cancelled" | "canceled"
        ) && self.ended.insert(id.into())
        {
            let failed = status != "completed";
            let result = if failed {
                state["error"].clone()
            } else {
                state["output"].clone()
            };
            let mut event = Event::new(
                id,
                end,
                Kind::ToolEnded {
                    result,
                    failed,
                    denied: denied(&state["error"]),
                },
            );
            event.duration_nanos = start
                .zip(end)
                .and_then(|(start, end)| end.checked_sub(start));
            events.push(event);
            if failed {
                let reason = state["error"].as_str().unwrap_or(status);
                events.push(Event::error(id, end, reason));
                if matches!(status, "cancelled" | "canceled") || interrupted(reason) {
                    events.push(Event::new(
                        id,
                        end,
                        Kind::Killed {
                            description: reason.into(),
                        },
                    ));
                }
            }
            if name == "task" {
                events.push(Event::new(
                    format!("{id}:agent"),
                    end,
                    Kind::AgentEnded { failed },
                ));
            }
        }
        events
    }
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
        reasoning,
        cost: message["cost"].as_f64(),
    }
}
