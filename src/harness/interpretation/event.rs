//! Harness-independent facts. IDs are scoped to the supplying session/run.
//! Missing times and usage stay unknown; decoding never invents measurements.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub time_unix_nanos: Option<u64>,
    pub duration_nanos: Option<u64>,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    ModelStarted {
        model: Option<String>,
    },
    ModelEnded {
        model: Option<String>,
        usage: Usage,
        failed: bool,
    },
    ToolStarted {
        name: String,
        input: Value,
    },
    ToolEnded {
        result: Value,
        failed: bool,
        denied: bool,
    },
    Message {
        role: String,
        text: String,
    },
    Error {
        kind: ErrorKind,
        message: String,
    },
    Killed {
        description: String,
    },
    AgentStarted {
        name: String,
    },
    AgentEnded {
        failed: bool,
    },
    /// Session totals, never an additional model call's usage.
    UsageSummary(Usage),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub reasoning: Option<u64>,
    pub cost: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ErrorKind {
    Other,
    UsageLimit,
    RateLimit,
}

impl Event {
    fn same_fact(&self, other: &Self) -> bool {
        self.id == other.id
            && std::mem::discriminant(&self.kind) == std::mem::discriminant(&other.kind)
    }

    pub(in crate::harness) fn new(
        id: impl Into<String>,
        time_unix_nanos: Option<u64>,
        kind: Kind,
    ) -> Self {
        Self {
            id: id.into(),
            time_unix_nanos,
            duration_nanos: None,
            kind,
        }
    }

    pub(in crate::harness) fn error(
        id: impl Into<String>,
        time: Option<u64>,
        message: &str,
    ) -> Self {
        let lower = message.to_lowercase();
        let kind = if lower.contains("rate") && lower.contains("limit") || lower.contains("429") {
            ErrorKind::RateLimit
        } else if ["quota", "usage limit", "usage_limit", "resource_exhausted"]
            .iter()
            .any(|word| lower.contains(word))
        {
            ErrorKind::UsageLimit
        } else {
            ErrorKind::Other
        };
        Self::new(
            id,
            time,
            Kind::Error {
                kind,
                message: message.into(),
            },
        )
    }
}

/// Retained records enrich matching live facts in place. New facts keep the
/// supplying record order. Call IDs plus phases prevent counting a call twice.
pub(in crate::harness) fn reconcile(live: &mut Vec<Event>, retained: Vec<Event>) {
    let mut cursor = None;
    for (position, mut event) in retained.iter().cloned().enumerate() {
        if let Some(index) = live.iter().position(|old| old.same_fact(&event)) {
            let old = &mut live[index];
            event.time_unix_nanos = event.time_unix_nanos.or(old.time_unix_nanos);
            event.duration_nanos = event.duration_nanos.or(old.duration_nanos);
            match (&mut event.kind, &old.kind) {
                (Kind::ModelStarted { model }, Kind::ModelStarted { model: previous }) => {
                    *model = model.take().or_else(|| previous.clone());
                }
                (
                    Kind::ModelEnded { model, usage, .. },
                    Kind::ModelEnded {
                        model: previous,
                        usage: old,
                        ..
                    },
                ) => {
                    *model = model.take().or_else(|| previous.clone());
                    usage.input = usage.input.or(old.input);
                    usage.output = usage.output.or(old.output);
                    usage.cache_read = usage.cache_read.or(old.cache_read);
                    usage.cache_write = usage.cache_write.or(old.cache_write);
                    usage.reasoning = usage.reasoning.or(old.reasoning);
                    usage.cost = usage.cost.or(old.cost);
                }
                (
                    Kind::ToolStarted { input, .. },
                    Kind::ToolStarted {
                        input: previous, ..
                    },
                ) if input.is_null() => {
                    *input = previous.clone();
                }
                (
                    Kind::ToolEnded { result, .. },
                    Kind::ToolEnded {
                        result: previous, ..
                    },
                ) if result.is_null() => {
                    *result = previous.clone();
                }
                _ => {}
            }
            *old = event;
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

pub(in crate::harness) fn nanos(value: &Value, multiplier: u64) -> Option<u64> {
    value.as_u64()?.checked_mul(multiplier)
}

pub(in crate::harness) fn denied(error: &Value) -> bool {
    error.to_string().to_lowercase().contains("denied")
}

pub(in crate::harness) fn interrupted(message: &str) -> bool {
    let message = message.to_lowercase();
    ["interrupt", "killed", "cancelled", "canceled"]
        .iter()
        .any(|word| message.contains(word))
}
