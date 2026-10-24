//! Session trace calls: one lifecycle shared by recording and the live Waterfall.
//! A call keeps its identity and parent from first observation through
//! completion and late amendments. No filesystem, clock or output lives here.
use std::collections::BTreeMap;

use super::{event_attributes, set};
use crate::harness::interpretation::event::{Event, Outcome};
use crate::logs::trace::{SpanKind, id};
use crate::otlp_jsonl::{KeyValue, SpanId};

/// A retained span ID that overrides the derived identity of one call, such as
/// a migrated or custom ID read back from a historical trace.
pub(crate) struct Retained<'a> {
    pub kind: SpanKind,
    pub id: &'a str,
    pub parent: Option<&'a str>,
    pub span: SpanId,
}

/// The normalized facts of one call. Times are the raw facts supplied with
/// events; each adapter performs its own conversion and clamping.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    pub span: SpanId,
    pub parent_span: SpanId,
    pub kind: SpanKind,
    pub id: String,
    pub parent_id: Option<String>,
    /// A tool call's tool, as first observed.
    pub tool: Option<String>,
    pub started: Option<u64>,
    pub ended: Option<u64>,
    pub completed: bool,
    /// The latest completion outcome. A closed call has none.
    pub outcome: Option<Outcome>,
    /// The last non-success completion, kept even after a later success.
    pub failure: Option<Outcome>,
    pub attributes: Vec<KeyValue>,
}

/// The call an event resolved to, as it stands after that event.
pub(crate) struct Applied {
    pub call: Call,
    /// Whether this event completed the call for the first time. Repeated
    /// completions and late amendments refine it without completing it again.
    pub first_completion: bool,
}

/// One Session's calls, keyed by the causal key of kind, parent and raw ID.
pub(crate) struct Calls {
    session: SpanId,
    input_excludes_cache: bool,
    retained: BTreeMap<String, SpanId>,
    locations: BTreeMap<String, String>,
    calls: BTreeMap<String, Call>,
}

impl Calls {
    pub fn new<'a>(
        session: SpanId,
        input_excludes_cache: bool,
        retained: impl IntoIterator<Item = Retained<'a>>,
    ) -> Self {
        Self {
            session,
            input_excludes_cache,
            retained: retained
                .into_iter()
                .map(|retained| {
                    (
                        key(retained.kind, retained.parent, retained.id),
                        retained.span,
                    )
                })
                .collect(),
            locations: BTreeMap::new(),
            calls: BTreeMap::new(),
        }
    }

    /// Apply one normalized event observed at `time`. Events that are not
    /// calls return nothing.
    pub fn apply(&mut self, event: &Event, time: Option<u64>) -> Option<Applied> {
        let fact = Fact::of(event)?;
        // A sub-agent completion names no parent; it ends its earlier start.
        let key = if fact.kind == SpanKind::Agent && fact.completion.is_some() {
            self.locations.get(fact.id).cloned()
        } else {
            None
        }
        .unwrap_or_else(|| key(fact.kind, fact.parent, fact.id));
        let parent_span = fact
            .parent
            .and_then(|parent| self.locate(parent))
            .unwrap_or(self.session);
        self.locations.insert(fact.id.to_owned(), key.clone());
        let span = self.identity(&key);
        let call = self.calls.entry(key).or_insert_with(|| Call {
            span,
            parent_span,
            kind: fact.kind,
            id: fact.id.to_owned(),
            parent_id: fact.parent.map(str::to_owned),
            tool: fact.tool.map(str::to_owned),
            started: time,
            ended: None,
            completed: false,
            outcome: None,
            failure: None,
            attributes: Vec::new(),
        });
        for attr in event_attributes(event, self.input_excludes_cache) {
            set(&mut call.attributes, attr);
        }
        let first_completion = fact.completion.is_some() && !call.completed;
        if let Some(outcome) = fact.completion {
            call.completed = true;
            call.ended = call.ended.or(time);
            call.outcome = Some(outcome);
            if outcome != Outcome::Succeeded {
                call.failure = Some(outcome);
            }
        }
        Some(Applied {
            call: call.clone(),
            first_completion,
        })
    }

    /// The span of the latest call with this raw ID, for log association.
    pub fn locate(&self, id: &str) -> Option<SpanId> {
        self.locations.get(id).map(|key| self.identity(key))
    }

    /// Every call observed so far, in causal-key order.
    pub fn calls(&self) -> impl Iterator<Item = &Call> {
        self.calls.values()
    }

    /// Complete every unfinished call at `time`, without an outcome.
    pub fn close(&mut self, time: u64) -> Vec<Call> {
        self.calls
            .values_mut()
            .filter(|call| !call.completed)
            .map(|call| {
                call.completed = true;
                call.ended = Some(time);
                call.clone()
            })
            .collect()
    }

    fn identity(&self, key: &str) -> SpanId {
        self.retained
            .get(key)
            .copied()
            .unwrap_or_else(|| id(&[&self.session.0, key.as_bytes()]))
    }
}

fn key(kind: SpanKind, parent: Option<&str>, id: &str) -> String {
    serde_json::to_string(&(kind, parent, id)).unwrap()
}

/// The call named by one normalized event, including Harness-local IDs scoped
/// by their parent call.
struct Fact<'a> {
    kind: SpanKind,
    id: &'a str,
    parent: Option<&'a str>,
    tool: Option<&'a str>,
    completion: Option<Outcome>,
}

impl<'a> Fact<'a> {
    fn of(event: &'a Event) -> Option<Self> {
        use Event::*;
        let tool = match event {
            ToolCallStarted { tool, .. } | ToolCallEnded { tool, .. } => Some(tool.as_str()),
            _ => None,
        };
        let (kind, id, parent, completion) = match event {
            ModelCallStarted { id, parent_id, .. } => {
                (SpanKind::Model, id, parent_id.as_deref(), None)
            }
            ModelCallEnded {
                id,
                parent_id,
                outcome,
                ..
            } => (SpanKind::Model, id, parent_id.as_deref(), Some(*outcome)),
            ToolCallStarted { id, parent_id, .. } => {
                (SpanKind::Tool, id, parent_id.as_deref(), None)
            }
            ToolCallEnded {
                id,
                parent_id,
                outcome,
                ..
            } => (SpanKind::Tool, id, parent_id.as_deref(), Some(*outcome)),
            TurnStarted { id } => (SpanKind::Turn, id, None, None),
            TurnEnded { id, outcome, .. } => (SpanKind::Turn, id, None, Some(*outcome)),
            SubagentStarted { id, parent_id, .. } => {
                (SpanKind::Agent, id, parent_id.as_deref(), None)
            }
            SubagentEnded { id, outcome, .. } => (SpanKind::Agent, id, None, Some(*outcome)),
            _ => return None,
        };
        Some(Self {
            kind,
            id,
            parent,
            tool,
            completion,
        })
    }
}

#[cfg(test)]
mod tests;
