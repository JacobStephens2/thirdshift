//! Codex exec's items have tool boundaries, but only whole-turn model usage.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::stream::{ItemKind, reports_running};
use crate::harness::interpretation::event::{Agents, ErrorKind, Event, Outcome, Tools, Usage};

#[derive(Default)]
pub(super) struct Events {
    tools: Tools,
    agents: Agents,
    turn: u64,
    messages: HashSet<String>,
}

impl Events {
    pub(super) fn decode(&mut self, event: &Value) -> Vec<Event> {
        let mut output = Vec::new();
        match event["type"].as_str() {
            Some("turn.started") => {
                self.turn += 1;
                output.push(Event::TurnStarted { id: self.turn_id() });
            }
            Some("turn.completed" | "turn.failed") => {
                let outcome = if event["type"] == "turn.completed" {
                    Outcome::Succeeded
                } else if event["error"]["message"]
                    .as_str()
                    .is_some_and(|text| text.to_ascii_lowercase().contains("interrupt"))
                {
                    Outcome::Interrupted
                } else {
                    Outcome::Failed
                };
                if event["type"] == "turn.failed" {
                    output.push(error(&event["error"]));
                }
                let stopped = if outcome == Outcome::Succeeded {
                    Outcome::Killed
                } else {
                    outcome
                };
                output.extend(self.tools.stop(stopped));
                output.extend(self.agents.stop(stopped));
                let usage = &event["usage"];
                output.push(Event::TurnEnded {
                    id: self.turn_id(),
                    usage: Usage {
                        input: usage["input_tokens"].as_u64(),
                        output: usage["output_tokens"].as_u64(),
                        cache_read: usage["cached_input_tokens"].as_u64(),
                        cache_write: usage["cache_write_input_tokens"].as_u64(),
                    },
                    outcome,
                });
            }
            Some("error") => output.push(error(event)),
            Some("item.started" | "item.updated" | "item.completed") => {
                self.item(event, &mut output)
            }
            _ => {}
        }
        output
    }

    fn turn_id(&self) -> String {
        format!("turn-{}", self.turn)
    }

    fn item(&mut self, event: &Value, output: &mut Vec<Event>) {
        let item = &event["item"];
        let Some(id) = item["id"].as_str() else {
            return;
        };
        let completed = event["type"] == "item.completed" && !reports_running(item);
        let kind = ItemKind::read(item);
        match kind {
            ItemKind::AgentMessage | ItemKind::Reasoning
                if completed && self.messages.insert(id.into()) =>
            {
                if let Some(text) = item["text"].as_str() {
                    output.push(Event::Message {
                        id: None,
                        role: "assistant".into(),
                        text: text.into(),
                        parent_id: None,
                    });
                }
                return;
            }
            ItemKind::Error if completed && self.messages.insert(id.into()) => {
                output.push(error(item));
                return;
            }
            ItemKind::Command
            | ItemKind::FileChange
            | ItemKind::McpTool
            | ItemKind::WebSearch
            | ItemKind::CollabTool
            | ItemKind::AgentMessage
            | ItemKind::Reasoning
            | ItemKind::TodoList
            | ItemKind::Error
            | ItemKind::Unknown => {}
        }
        let (tool, input, result) = match kind {
            ItemKind::Command => (
                "command_execution".into(),
                json!({"command":item["command"]}),
                json!({"output":item["aggregated_output"],"exit_code":item["exit_code"]}),
            ),
            ItemKind::FileChange => (
                "apply_patch".into(),
                item["changes"].clone(),
                item["changes"].clone(),
            ),
            ItemKind::McpTool => (
                format!(
                    "{}.{}",
                    item["server"].as_str().unwrap_or("?"),
                    item["tool"].as_str().unwrap_or("?")
                ),
                item["arguments"].clone(),
                if item["error"].is_null() {
                    item["result"].clone()
                } else {
                    item["error"].clone()
                },
            ),
            ItemKind::WebSearch => (
                "web_search".into(),
                json!({"query":item["query"],"action":item["action"]}),
                item["results"].clone(),
            ),
            ItemKind::CollabTool => (
                item["tool"].as_str().unwrap_or("?").into(),
                json!({"prompt":item["prompt"],"receiver_thread_ids":item["receiver_thread_ids"],"sender_thread_id":item["sender_thread_id"]}),
                item["agents_states"].clone(),
            ),
            ItemKind::TodoList => (
                "update_plan".into(),
                item["items"].clone(),
                item["items"].clone(),
            ),
            ItemKind::AgentMessage | ItemKind::Reasoning | ItemKind::Error | ItemKind::Unknown => {
                return;
            }
        };
        output.extend(self.tools.start(id, None, tool, input));
        if completed {
            let outcome = match item["status"].as_str() {
                Some("declined" | "denied") => Outcome::Denied,
                Some("interrupted") => Outcome::Interrupted,
                Some("killed") => Outcome::Killed,
                Some("failed") => Outcome::Failed,
                _ if !item["error"].is_null()
                    || item["exit_code"].as_i64().is_some_and(|code| code != 0) =>
                {
                    Outcome::Failed
                }
                _ => Outcome::Succeeded,
            };
            output.extend(self.tools.end(id, None, result, outcome));
        }
        if kind == ItemKind::CollabTool {
            self.agents(item, output);
        }
    }

    fn agents(&mut self, item: &Value, output: &mut Vec<Event>) {
        let parent_id = item["sender_thread_id"].as_str().map(String::from);
        let description = item["prompt"].as_str().map(String::from);
        if item["tool"] == "spawn_agent" && item["status"] == "completed" {
            for id in item["receiver_thread_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                output.extend(
                    self.agents
                        .start(id, parent_id.clone(), description.clone()),
                );
            }
        }
        for (id, state) in item["agents_states"].as_object().into_iter().flatten() {
            let outcome = match state["status"].as_str() {
                Some("completed") => Some(Outcome::Succeeded),
                Some("errored") => Some(Outcome::Failed),
                Some("interrupted") => Some(Outcome::Interrupted),
                Some("shutdown") => Some(Outcome::Killed),
                Some("running" | "pending_init") => None,
                _ => continue,
            };
            output.extend(
                self.agents
                    .start(id, parent_id.clone(), description.clone()),
            );
            if let Some(outcome) = outcome {
                output.extend(self.agents.end(id, outcome, state["message"].clone()));
            }
        }
    }
}

fn error(event: &Value) -> Event {
    let message = event["message"].as_str().unwrap_or("Codex error");
    Event::Error {
        kind: ErrorKind::from_text(message),
        message: message.into(),
        parent_id: None,
        details: event.clone(),
    }
}
