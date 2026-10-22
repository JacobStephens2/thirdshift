//! Muse task IDs bridge lifecycle edges and the later call/result records.
use crate::harness::interpretation::event::{Event, Outcome, Usage, denied, interrupted};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Events {
    tasks: HashMap<String, Task>,
    calls: HashMap<String, String>,
    models: HashMap<String, String>,
    last_model: HashMap<String, String>,
    recovered_calls: usize,
    text: HashMap<String, String>,
}

struct Task {
    kind: TaskKind,
    started: bool,
    ended: bool,
    output: Value,
    input: Value,
}

enum TaskKind {
    Model,
    Tool(String),
    Agent(String),
}

impl Task {
    fn new(kind: TaskKind) -> Self {
        Self {
            kind,
            started: false,
            ended: false,
            input: Value::Null,
            output: Value::Null,
        }
    }

    fn start(&mut self, id: &str, model: Option<String>) -> Option<Event> {
        if self.started {
            return None;
        }
        self.started = true;
        Some(match &self.kind {
            TaskKind::Model => Event::ModelCallStarted {
                id: id.into(),
                parent_id: None,
                model,
            },
            TaskKind::Tool(tool) => Event::ToolCallStarted {
                id: id.into(),
                parent_id: None,
                tool: tool.clone(),
                input: self.input.clone(),
            },
            TaskKind::Agent(name) => Event::SubagentStarted {
                id: id.into(),
                parent_id: None,
                description: Some(name.clone()),
            },
        })
    }

    fn end(&mut self, id: &str, model: Option<String>, outcome: Outcome) -> Event {
        let amended = self.ended;
        self.ended = true;
        match &self.kind {
            TaskKind::Model => Event::ModelCallEnded {
                id: id.into(),
                parent_id: None,
                model,
                usage: Usage::default(),
                cost_usd: None,
                outcome,
            },
            TaskKind::Tool(tool) => Event::ToolCallEnded {
                id: id.into(),
                parent_id: None,
                tool: tool.clone(),
                input: self.input.clone(),
                result: self.output.clone(),
                outcome,
                amended,
            },
            TaskKind::Agent(_) => Event::SubagentEnded {
                id: id.into(),
                outcome,
                result: self.output.clone(),
            },
        }
    }
}

impl Events {
    pub(super) fn stream(&mut self, event: &Value) -> Vec<Event> {
        let payload = &event["payload"];
        let scope = payload["run_stream"]["id"]
            .as_str()
            .or_else(|| payload["command_id"].as_str())
            .or_else(|| event["stream"]["id"].as_str())
            .unwrap_or("");
        let kind = event["payload_type"].as_str().unwrap_or("");
        if kind == "run.model.configured" {
            if let Some(model) = payload["model_id"].as_str() {
                self.models.insert(scope.into(), model.into());
            }
            return Vec::new();
        }
        if let Some(kind) = kind.strip_prefix("task.lifecycle.") {
            return self.lifecycle(scope, kind, &payload["event"]);
        }
        if kind == "tool.result" {
            return self.result(scope, payload);
        }
        if let Some(status) = kind.strip_prefix("run.terminal.") {
            return self.terminal(scope, status, payload);
        }
        if kind == "turn.input.received" {
            return payload["text"]
                .as_str()
                .map(|text| Event::Message {
                    id: None,
                    role: "user".into(),
                    text: text.into(),
                    parent_id: None,
                })
                .into_iter()
                .collect();
        }
        if kind == "run.output.delta" {
            let id = self.reply_id(scope);
            let text = self.text.entry(id.clone()).or_default();
            text.push_str(payload["text"].as_str().unwrap_or(""));
            return vec![Event::Message {
                id: Some(format!("{id}:message")),
                role: "assistant".into(),
                text: text.clone(),
                parent_id: None,
            }];
        }
        Vec::new()
    }

    pub(super) fn record(&mut self, record: &Value) -> Vec<Event> {
        let payload = &record["payload"];
        let scope = payload["run_id"]
            .as_str()
            .or_else(|| record["stream"]["id"].as_str())
            .unwrap_or("");
        let event = &payload["event"];
        match event["kind"].as_str().unwrap_or("") {
            "terminal" => self.terminal(scope, event["terminal"].as_str().unwrap_or(""), event),
            "model_completed" => {
                let mut events = Vec::new();
                let id = self
                    .last_model
                    .get(scope)
                    .filter(|id| self.tasks.contains_key(*id))
                    .cloned()
                    .unwrap_or_else(|| {
                        self.recovered_calls += 1;
                        format!("{scope}:model:{}", self.recovered_calls)
                    });
                let model = event["model"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| self.models.get(scope).cloned());
                if !self.tasks.contains_key(&id) {
                    events.push(Event::ModelCallStarted {
                        id: id.clone(),
                        parent_id: None,
                        model: model.clone(),
                    });
                }
                self.last_model.insert(scope.into(), id.clone());
                let usage = &event["usage"];
                events.push(Event::ModelCallEnded {
                    id,
                    parent_id: None,
                    model,
                    usage: Usage {
                        input: usage["input_tokens"].as_u64(),
                        output: usage["output_tokens"].as_u64(),
                        cache_read: usage["cached_tokens"].as_u64(),
                        cache_write: usage["cache_write_tokens"].as_u64(),
                    },
                    cost_usd: event["cost"].as_f64(),
                    outcome: Outcome::Succeeded,
                });
                events
            }
            "assistant_message_committed" => {
                let id = self.last_model.get(scope).cloned().unwrap_or_else(|| {
                    format!(
                        "{scope}:{}",
                        event["message_id"].as_str().unwrap_or("reply")
                    )
                });
                event["text"]
                    .as_str()
                    .map(|text| Event::Message {
                        id: Some(format!("{id}:message")),
                        role: "assistant".into(),
                        text: text.into(),
                        parent_id: None,
                    })
                    .into_iter()
                    .collect()
            }
            kind => self.lifecycle(scope, kind, event),
        }
    }

    fn reply_id(&self, scope: &str) -> String {
        self.last_model
            .get(scope)
            .cloned()
            .unwrap_or_else(|| format!("{scope}:reply"))
    }

    fn terminal(&self, scope: &str, status: &str, payload: &Value) -> Vec<Event> {
        if status == "completed" {
            let id = self.reply_id(scope);
            return payload["text"]
                .as_str()
                .filter(|_| !self.text.contains_key(&id))
                .map(|text| Event::Message {
                    id: Some(format!("{id}:message")),
                    role: "assistant".into(),
                    text: text.into(),
                    parent_id: None,
                })
                .into_iter()
                .collect();
        }
        terminal(scope, status, payload)
    }

    fn lifecycle(&mut self, scope: &str, kind: &str, event: &Value) -> Vec<Event> {
        let Some(task_id) = event["task_id"].as_str() else {
            return Vec::new();
        };
        let id = format!("{scope}:{task_id}");
        if kind == "proposed" {
            let kind = event["task_kind"].as_str().unwrap_or("");
            let kind = if kind.starts_with("model.") {
                TaskKind::Model
            } else if let Some(name) = kind.strip_prefix("tool.") {
                TaskKind::Tool(name.into())
            } else if kind.contains("agent.") {
                TaskKind::Agent(kind.into())
            } else {
                return Vec::new();
            };
            self.tasks.entry(id).or_insert_with(|| Task::new(kind));
            return Vec::new();
        }
        if let Some(call) = event["idempotency_key"]
            .as_str()
            .and_then(|key| key.strip_prefix("tool:"))
        {
            self.calls.insert(format!("{scope}:{call}"), id.clone());
        }
        let model = self.models.get(scope).cloned();
        let Some(task) = self.tasks.get_mut(&id) else {
            return Vec::new();
        };
        if kind == "started" && matches!(task.kind, TaskKind::Model) {
            self.last_model.insert(scope.into(), id.clone());
        }
        match kind {
            "started" => task.start(&id, model).into_iter().collect(),
            "output" => {
                if let Some(chunk) = event["chunk"].as_str() {
                    let mut text = task.output.as_str().unwrap_or("").to_string();
                    text.push_str(chunk);
                    task.output = Value::String(text);
                }
                Vec::new()
            }
            "completed" | "failed" | "cancelled" => {
                if task.ended {
                    return Vec::new();
                }
                let mut events: Vec<_> = task.start(&id, model.clone()).into_iter().collect();
                let reason = event["reason"].as_str().unwrap_or(kind);
                let outcome = if kind == "completed" {
                    Outcome::Succeeded
                } else if kind == "cancelled" {
                    Outcome::Interrupted
                } else if denied(&event["reason"]) {
                    Outcome::Denied
                } else {
                    Outcome::Failed
                };
                if kind != "completed" {
                    task.output = Value::String(reason.into());
                }
                events.push(task.end(&id, model, outcome));
                if kind != "completed" {
                    events.push(Event::error(&id, reason, event.clone()));
                }
                if kind == "cancelled" {
                    events.push(Event::WorkEnded { id, outcome });
                }
                events
            }
            _ => Vec::new(),
        }
    }

    fn result(&mut self, scope: &str, payload: &Value) -> Vec<Event> {
        let Some(call_id) = payload["call_id"].as_str() else {
            return Vec::new();
        };
        let call = format!("{scope}:{call_id}");
        let id = self.calls.get(&call).cloned().unwrap_or(call);
        let name = payload["correlation_facts"]["tool_name"]
            .as_str()
            .unwrap_or("tool");
        let task = self
            .tasks
            .entry(id.clone())
            .or_insert_with(|| Task::new(TaskKind::Tool(name.into())));
        let result = payload["text"].clone();
        if let Some(text) = result
            .as_str()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            && let Some(command) = text.get("command")
        {
            task.input = serde_json::json!({"command": command});
        }
        task.output = result;
        let reported = &payload["correlation_facts"]["outcome"];
        let outcome = if denied(reported) {
            Outcome::Denied
        } else if reported == "success" {
            Outcome::Succeeded
        } else {
            Outcome::Failed
        };
        let mut events: Vec<_> = task.start(&id, None).into_iter().collect();
        events.push(task.end(&id, None, outcome));
        events
    }
}

fn terminal(scope: &str, status: &str, payload: &Value) -> Vec<Event> {
    if !matches!(status, "failed" | "cancelled" | "interrupted") {
        return Vec::new();
    }
    let reason = payload["reason"].as_str().unwrap_or("Muse's turn failed");
    let id = format!("{scope}:terminal");
    let mut events = vec![Event::error(&id, reason, payload.clone())];
    if status != "failed" || interrupted(reason) {
        events.push(Event::WorkEnded {
            id,
            outcome: Outcome::Interrupted,
        });
    }
    events
}
