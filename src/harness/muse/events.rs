//! Muse task IDs bridge lifecycle edges and the later call/result records.
use crate::harness::interpretation::event::{Event, Kind, Usage, denied, nanos};
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
    start: Option<u64>,
    started: bool,
    output: Value,
    input: Value,
    denied: bool,
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
            start: None,
            started: false,
            input: Value::Null,
            output: Value::Null,
            denied: false,
        }
    }

    fn start(&mut self, id: &str, time: Option<u64>, model: Option<String>) -> Event {
        if !self.started {
            self.start = time;
            self.started = true;
        }
        Event::new(
            id,
            self.start,
            match &self.kind {
                TaskKind::Model => Kind::ModelStarted { model },
                TaskKind::Tool(name) => Kind::ToolStarted {
                    name: name.clone(),
                    input: self.input.clone(),
                },
                TaskKind::Agent(name) => Kind::AgentStarted { name: name.clone() },
            },
        )
    }

    fn end(&self, id: &str, time: Option<u64>, model: Option<String>, failed: bool) -> Event {
        Event::new(
            id,
            time,
            match &self.kind {
                TaskKind::Model => Kind::ModelEnded {
                    model,
                    usage: Usage::default(),
                    failed,
                },
                TaskKind::Tool(_) => Kind::ToolEnded {
                    result: self.output.clone(),
                    failed,
                    denied: self.denied,
                },
                TaskKind::Agent(_) => Kind::AgentEnded { failed },
            },
        )
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
        let time = nanos(&event["recorded_at"], 1000);
        let kind = event["payload_type"].as_str().unwrap_or("");
        if kind == "run.model.configured" {
            if let Some(model) = payload["model_id"].as_str() {
                self.models.insert(scope.into(), model.into());
            }
            return Vec::new();
        }
        if let Some(kind) = kind.strip_prefix("task.lifecycle.") {
            return self.lifecycle(scope, kind, &payload["event"], time);
        }
        if kind == "tool.result" {
            return self.result(scope, payload, time);
        }
        if matches!(
            kind,
            "run.terminal.failed" | "run.terminal.cancelled" | "run.terminal.interrupted"
        ) {
            return terminal(
                scope,
                kind.strip_prefix("run.terminal.").unwrap(),
                payload,
                time,
            );
        }
        if kind == "turn.input.user" {
            return payload["prompt"]
                .as_str()
                .map(|text| {
                    Event::new(
                        event["id"].as_str().unwrap_or(scope),
                        time,
                        Kind::Message {
                            role: "user".into(),
                            text: text.into(),
                        },
                    )
                })
                .into_iter()
                .collect();
        }
        if kind == "run.output.delta" {
            let id = self
                .last_model
                .get(scope)
                .cloned()
                .unwrap_or_else(|| format!("{scope}:reply"));
            let text = self.text.entry(id.clone()).or_default();
            text.push_str(payload["text"].as_str().unwrap_or(""));
            return vec![Event::new(
                format!("{id}:message"),
                time,
                Kind::Message {
                    role: "assistant".into(),
                    text: text.clone(),
                },
            )];
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
        let time = nanos(&record["recorded_at"], 1000);
        match event["kind"].as_str().unwrap_or("") {
            "terminal" => terminal(scope, event["terminal"].as_str().unwrap_or(""), event, time),
            "model_completed" => {
                let mut events = Vec::new();
                let id = self.last_model.get(scope).cloned().unwrap_or_else(|| {
                    self.recovered_calls += 1;
                    format!("{scope}:model:{}", self.recovered_calls)
                });
                let model = event["model"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| self.models.get(scope).cloned());
                if !self.tasks.contains_key(&id) {
                    let start = time
                        .zip(nanos(&event["duration_ms"], 1_000_000))
                        .and_then(|(end, duration)| end.checked_sub(duration));
                    events.push(Event::new(
                        &id,
                        start,
                        Kind::ModelStarted {
                            model: model.clone(),
                        },
                    ));
                }
                let usage = &event["usage"];
                let mut end = Event::new(
                    &id,
                    time,
                    Kind::ModelEnded {
                        model,
                        usage: Usage {
                            input: usage["input_tokens"].as_u64(),
                            output: usage["output_tokens"].as_u64(),
                            cache_read: usage["cached_tokens"].as_u64(),
                            cache_write: usage["cache_write_tokens"].as_u64(),
                            reasoning: usage["reasoning_tokens"].as_u64(),
                            cost: event["cost"].as_f64(),
                        },
                        failed: false,
                    },
                );
                end.duration_nanos = nanos(&event["duration_ms"], 1_000_000);
                events.push(end);
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
                    .map(|text| {
                        Event::new(
                            format!("{id}:message"),
                            time,
                            Kind::Message {
                                role: "assistant".into(),
                                text: text.into(),
                            },
                        )
                    })
                    .into_iter()
                    .collect()
            }
            kind => self.lifecycle(scope, kind, event, time),
        }
    }

    fn lifecycle(
        &mut self,
        scope: &str,
        kind: &str,
        event: &Value,
        time: Option<u64>,
    ) -> Vec<Event> {
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
            "started" => vec![task.start(&id, time, model)],
            "output" => {
                if let Some(chunk) = event["chunk"].as_str() {
                    let mut text = task.output.as_str().unwrap_or("").to_string();
                    text.push_str(chunk);
                    task.output = Value::String(text);
                }
                Vec::new()
            }
            "completed" | "failed" | "cancelled" => {
                let mut events = Vec::new();
                if !task.started {
                    events.push(task.start(&id, None, model.clone()));
                }
                let reason = event["reason"].as_str().unwrap_or(kind);
                task.denied = denied(&event["reason"]);
                if kind != "completed" {
                    task.output = Value::String(reason.into());
                }
                events.push(task.end(&id, time, model, kind != "completed"));
                if kind != "completed" {
                    events.push(Event::error(&id, time, reason));
                }
                if kind == "cancelled" {
                    events.push(Event::new(
                        id,
                        time,
                        Kind::Killed {
                            description: reason.into(),
                        },
                    ));
                }
                events
            }
            _ => Vec::new(),
        }
    }

    fn result(&mut self, scope: &str, payload: &Value, time: Option<u64>) -> Vec<Event> {
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
            task.input = serde_json::json!({"command":command});
        }
        task.output = result;
        task.denied = denied(&payload["correlation_facts"]["outcome"]);
        vec![
            task.start(&id, time, None),
            task.end(
                &id,
                time,
                None,
                payload["correlation_facts"]["outcome"] != "success",
            ),
        ]
    }
}

fn terminal(scope: &str, status: &str, payload: &Value, time: Option<u64>) -> Vec<Event> {
    if !matches!(status, "failed" | "cancelled" | "interrupted") {
        return Vec::new();
    }
    let reason = payload["reason"].as_str().unwrap_or("Muse's turn failed");
    let id = format!("{scope}:terminal");
    let mut events = vec![Event::error(&id, time, reason)];
    if status != "failed" || crate::harness::interpretation::event::interrupted(reason) {
        events.push(Event::new(
            id,
            time,
            Kind::Killed {
                description: reason.into(),
            },
        ));
    }
    events
}
