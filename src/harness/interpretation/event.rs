//! Harness-independent facts. Missing measurements stay unknown; session and
//! turn totals never masquerade as per-request usage. Boundaries are when the
//! stream reports them, which can be after the work (notably completed-only
//! Codex patches and Claude messages without partial streaming).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        /// Late authoritative metadata can refine a prior end. Consumers update
        /// that call's outcome without counting another call or moving its end.
        amended: bool,
    },
    Message {
        /// Stable message identity when reported. Snapshots with this ID refine
        /// the same message; parent_id is only its causal scope.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
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

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    Session,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Denied,
    Killed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Other,
    UsageLimit,
    RateLimit,
}

impl ErrorKind {
    pub(crate) fn from_text(text: &str) -> Self {
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

impl Event {
    pub(in crate::harness) fn error(parent_id: &str, message: &str, details: Value) -> Self {
        Self::Error {
            kind: ErrorKind::from_text(message),
            message: message.into(),
            parent_id: Some(parent_id.into()),
            details,
        }
    }

    fn same_fact(&self, other: &Self) -> bool {
        use Event::*;
        match (self, other) {
            (
                ModelCallStarted {
                    id: a,
                    parent_id: x,
                    ..
                },
                ModelCallStarted {
                    id: b,
                    parent_id: y,
                    ..
                },
            )
            | (
                ModelCallEnded {
                    id: a,
                    parent_id: x,
                    ..
                },
                ModelCallEnded {
                    id: b,
                    parent_id: y,
                    ..
                },
            )
            | (
                ToolCallStarted {
                    id: a,
                    parent_id: x,
                    ..
                },
                ToolCallStarted {
                    id: b,
                    parent_id: y,
                    ..
                },
            )
            | (
                ToolCallEnded {
                    id: a,
                    parent_id: x,
                    ..
                },
                ToolCallEnded {
                    id: b,
                    parent_id: y,
                    ..
                },
            ) => a == b && x == y,
            (SubagentStarted { id: a, .. }, SubagentStarted { id: b, .. })
            | (SubagentEnded { id: a, .. }, SubagentEnded { id: b, .. })
            | (WorkEnded { id: a, .. }, WorkEnded { id: b, .. }) => a == b,
            (
                Message {
                    id: Some(a),
                    parent_id: x,
                    ..
                },
                Message {
                    id: Some(b),
                    parent_id: y,
                    ..
                },
            ) => a == b && x == y,
            (Message { .. }, Message { .. }) => false,
            _ => self == other,
        }
    }

    fn enrich(&mut self, newer: Self) {
        match (self, newer) {
            (Self::ModelCallStarted { model, .. }, Self::ModelCallStarted { model: new, .. }) => {
                *model = new.or_else(|| model.take())
            }
            (
                Self::ModelCallEnded {
                    model,
                    usage,
                    cost_usd,
                    outcome,
                    ..
                },
                Self::ModelCallEnded {
                    model: new_model,
                    usage: new_usage,
                    cost_usd: new_cost,
                    outcome: new_outcome,
                    ..
                },
            ) => {
                *model = new_model.or_else(|| model.take());
                usage.merge(new_usage);
                *cost_usd = new_cost.or(*cost_usd);
                *outcome = new_outcome;
            }
            (Self::ToolCallStarted { input, .. }, Self::ToolCallStarted { input: new, .. }) => {
                if !new.is_null() {
                    *input = new;
                }
            }
            (
                Self::ToolCallEnded {
                    input,
                    result,
                    outcome,
                    amended,
                    ..
                },
                Self::ToolCallEnded {
                    input: new_input,
                    result: new_result,
                    outcome: new_outcome,
                    amended: new_amended,
                    ..
                },
            ) => {
                if !new_input.is_null() {
                    *input = new_input;
                }
                if !new_result.is_null() {
                    *result = new_result;
                }
                *outcome = new_outcome;
                *amended = new_amended;
            }
            (old, new) => *old = new,
        }
    }
}

/// Retained records refine matching live phases in place. Missing phases keep
/// their record order, anchored to the next known call when no predecessor is
/// known. This is an authoritative timeline, not another stream of call ends.
pub(in crate::harness) fn reconcile(live: &mut Vec<Event>, retained: Vec<Event>) {
    let mut cursor = None;
    for (position, event) in retained.iter().cloned().enumerate() {
        if let Event::ToolCallEnded { id, parent_id, input, .. } = &event
            && !input.is_null()
            && let Some(Event::ToolCallStarted { input: previous, .. }) = live.iter_mut().find(|old| matches!(old, Event::ToolCallStarted { id: call, parent_id: scope, .. } if call == id && scope == parent_id)) {
                *previous = input.clone();
            }
        if let Some(index) = live.iter().position(|old| old.same_fact(&event)) {
            live[index].enrich(event);
            cursor = Some(index + 1);
        } else {
            let index = cursor.unwrap_or_else(|| {
                retained[position + 1..]
                    .iter()
                    .find_map(|next| live.iter().position(|old| old.same_fact(next)))
                    .unwrap_or(live.len())
            });
            live.insert(index, event);
            cursor = Some(index + 1);
        }
    }
}

pub(in crate::harness) fn denied(error: &Value) -> bool {
    error.to_string().to_ascii_lowercase().contains("denied")
}

pub(in crate::harness) fn interrupted(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    ["interrupt", "killed", "cancelled", "canceled"]
        .iter()
        .any(|word| message.contains(word))
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
    ended: HashMap<(Option<String>, String), Outcome>,
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
            amended: false,
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
        if self.ended.contains_key(&(parent_id.clone(), id.into())) {
            return None;
        }
        if let Some(call) = self
            .running
            .iter_mut()
            .find(|call| call.parent_id == parent_id && call.id == id)
        {
            call.input = input;
            call.name = tool;
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
        self.ended.insert((parent_id, id.into()), outcome);
        Some(call.end(result, outcome))
    }

    pub(in crate::harness) fn deny(
        &mut self,
        id: &str,
        parent_id: Option<String>,
        tool: String,
        input: Value,
        result: Value,
    ) -> Vec<Event> {
        let key = (parent_id.clone(), id.into());
        if let Some(previous) = self.ended.get_mut(&key) {
            if *previous == Outcome::Denied {
                return Vec::new();
            }
            *previous = Outcome::Denied;
            return vec![Event::ToolCallEnded {
                id: id.into(),
                parent_id,
                tool,
                input,
                result,
                outcome: Outcome::Denied,
                amended: true,
            }];
        }
        let mut output: Vec<_> = self
            .start(id, parent_id.clone(), tool, input)
            .into_iter()
            .collect();
        output.extend(self.end(id, parent_id, result, Outcome::Denied));
        output
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
                self.ended
                    .insert((call.parent_id.clone(), id.clone()), outcome);
                output.push(call.end(Value::Null, outcome));
                output.push(Event::WorkEnded { id, outcome });
            } else {
                self.running.push(call);
            }
        }
        output
    }
}
