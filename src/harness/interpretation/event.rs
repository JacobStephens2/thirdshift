//! Harness-independent facts. Missing measurements stay unknown; session and
//! turn totals never masquerade as per-request usage. Boundaries are when the
//! stream reports them, which can be after the work (notably completed-only
//! Codex patches and Claude messages without partial streaming).

use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ModelCallStarted {
        id: String,
        parent_id: Option<String>,
        model: Option<String>,
    },
    ModelCallEnded {
        id: String,
        parent_id: Option<String>,
        model: Option<String>,
        usage: Usage,
        cost_usd: Option<f64>,
        outcome: Outcome,
    },
    /// Codex exec reports a whole turn, containing an unknown number of model
    /// requests. It supplies neither their boundaries nor their Models.
    TurnStarted {
        id: String,
    },
    TurnEnded {
        id: String,
        usage: Usage,
        outcome: Outcome,
    },
    ToolCallStarted {
        id: String,
        parent_id: Option<String>,
        tool: String,
        input: Value,
    },
    ToolCallEnded {
        id: String,
        parent_id: Option<String>,
        tool: String,
        input: Value,
        result: Value,
        outcome: Outcome,
    },
    Message {
        role: String,
        text: String,
        parent_id: Option<String>,
    },
    Usage {
        scope: UsageScope,
        model: Option<String>,
        usage: Usage,
        cost_usd: Option<f64>,
    },
    Error {
        kind: ErrorKind,
        message: String,
        parent_id: Option<String>,
        /// Structured reset times, codes and other Harness-supplied details.
        details: Value,
    },
    WorkEnded {
        id: String,
        outcome: Outcome,
    },
    SubagentStarted {
        id: String,
        parent_id: Option<String>,
        description: Option<String>,
    },
    SubagentEnded {
        id: String,
        outcome: Outcome,
        result: Value,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Usage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

impl Usage {
    pub(super) fn messages(value: &Value) -> Self {
        Self {
            input: value["input_tokens"].as_u64(),
            output: value["output_tokens"].as_u64(),
            cache_read: value["cache_read_input_tokens"].as_u64(),
            cache_write: value["cache_creation_input_tokens"].as_u64(),
        }
    }

    pub(super) fn merge(&mut self, newer: Self) {
        self.input = newer.input.or(self.input);
        self.output = newer.output.or(self.output);
        self.cache_read = newer.cache_read.or(self.cache_read);
        self.cache_write = newer.cache_write.or(self.cache_write);
    }

    pub(super) fn reported(&self) -> bool {
        self != &Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    Session,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Denied,
    Killed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Other,
    UsageLimit,
    RateLimit,
}

impl ErrorKind {
    pub(in crate::harness) fn from_text(text: &str) -> Self {
        let text = text.to_ascii_lowercase();
        if [
            "usage_limit",
            "usage limit",
            "quota",
            "spend limit",
            "hit your limit",
        ]
        .iter()
        .any(|term| text.contains(term))
        {
            Self::UsageLimit
        } else if text.contains("rate_limit") || text.contains("rate limit") || text.contains("429")
        {
            Self::RateLimit
        } else {
            Self::Other
        }
    }
}

/// Each sub-agent may appear in several tool snapshots or task notifications.
#[derive(Default)]
pub(in crate::harness) struct Agents {
    started: HashSet<String>,
    ended: HashSet<String>,
    running: Vec<String>,
}

impl Agents {
    pub(in crate::harness) fn start(
        &mut self,
        id: &str,
        parent_id: Option<String>,
        description: Option<String>,
    ) -> Option<Event> {
        if !self.started.insert(id.into()) {
            return None;
        }
        self.running.push(id.into());
        Some(Event::SubagentStarted {
            id: id.into(),
            parent_id,
            description,
        })
    }

    pub(in crate::harness) fn end(
        &mut self,
        id: &str,
        outcome: Outcome,
        result: Value,
    ) -> Option<Event> {
        if !self.started.contains(id) || !self.ended.insert(id.into()) {
            return None;
        }
        self.running.retain(|running| running != id);
        Some(Event::SubagentEnded {
            id: id.into(),
            outcome,
            result,
        })
    }

    pub(in crate::harness) fn stop(&mut self, outcome: Outcome) -> Vec<Event> {
        let mut output = Vec::new();
        for id in std::mem::take(&mut self.running) {
            output.extend(self.end(&id, outcome, Value::Null));
            output.push(Event::WorkEnded { id, outcome });
        }
        output
    }
}

/// Shared tool lifecycle rules; completion-only items can supply their start
/// at completion, while repeated snapshots never become extra calls.
#[derive(Default)]
pub(in crate::harness) struct Tools {
    seen: HashSet<(Option<String>, String)>,
    running: Vec<Tool>,
}

struct Tool {
    parent_id: Option<String>,
    id: String,
    name: String,
    input: Value,
}

impl Tool {
    fn end(self, result: Value, outcome: Outcome) -> Event {
        Event::ToolCallEnded {
            id: self.id,
            parent_id: self.parent_id,
            tool: self.name,
            input: self.input,
            result,
            outcome,
        }
    }
}

impl Tools {
    pub(in crate::harness) fn start(
        &mut self,
        id: &str,
        parent_id: Option<String>,
        tool: String,
        input: Value,
    ) -> Option<Event> {
        if !self.seen.insert((parent_id.clone(), id.into())) {
            if let Some(call) = self
                .running
                .iter_mut()
                .find(|call| call.parent_id == parent_id && call.id == id)
            {
                call.input = input;
                call.name = tool;
            }
            return None;
        }
        self.running.push(Tool {
            parent_id: parent_id.clone(),
            id: id.into(),
            name: tool.clone(),
            input: input.clone(),
        });
        Some(Event::ToolCallStarted {
            id: id.into(),
            parent_id,
            tool,
            input,
        })
    }

    pub(in crate::harness) fn end(
        &mut self,
        id: &str,
        parent_id: Option<String>,
        result: Value,
        outcome: Outcome,
    ) -> Option<Event> {
        let index = self
            .running
            .iter()
            .position(|call| call.parent_id == parent_id && call.id == id)?;
        let call = self.running.remove(index);
        Some(call.end(result, outcome))
    }

    pub(in crate::harness) fn stop(&mut self, outcome: Outcome) -> Vec<Event> {
        self.stop_matching(|_| true, outcome)
    }

    pub(in crate::harness) fn stop_scope(
        &mut self,
        parent_id: &Option<String>,
        outcome: Outcome,
    ) -> Vec<Event> {
        self.stop_matching(|call| &call.parent_id == parent_id, outcome)
    }

    fn stop_matching(&mut self, matches: impl Fn(&Tool) -> bool, outcome: Outcome) -> Vec<Event> {
        let mut output = Vec::new();
        for call in std::mem::take(&mut self.running) {
            if matches(&call) {
                let id = call.id.clone();
                output.push(call.end(Value::Null, outcome));
                output.push(Event::WorkEnded { id, outcome });
            } else {
                self.running.push(call);
            }
        }
        output
    }
}
