//! Measurements from one normalized Session trace, without Harness price tables.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::harness::interpretation::event::{Event, Outcome, Usage, UsageScope};
use crate::otlp_jsonl::{AnyValue, KeyValue, Logs};

mod destructive;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Signals {
    pub tokens_by_model: Vec<ModelTokens>,
    pub cost_usd: Option<f64>,
    pub tool_errors: u64,
    pub permission_denials: u64,
    pub killed_work: u64,
    pub repeated_tool_calls: u64,
    #[serde(default)]
    pub destructive_commands: Option<Vec<destructive::Flag>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct ModelTokens {
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SessionSignals {
    pub harness: Option<String>,
    pub harness_version: Option<String>,
    pub signals: Signals,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Summary {
    pub observed_sessions: usize,
    pub missing_sessions: usize,
    pub signals: Option<Signals>,
}

impl Summary {
    pub fn add(&mut self, session: Option<&SessionSignals>) {
        if let Some(session) = session {
            self.observed_sessions += 1;
            self.signals.get_or_insert_default().add(&session.signals);
        } else {
            self.missing_sessions += 1;
        }
    }
}

impl Signals {
    pub fn add(&mut self, other: &Self) {
        for tokens in &other.tokens_by_model {
            let index = self
                .tokens_by_model
                .iter()
                .position(|t| t.model == tokens.model);
            if let Some(index) = index {
                self.tokens_by_model[index].add(tokens);
            } else {
                self.tokens_by_model.push(tokens.clone());
            }
        }
        sum(&mut self.cost_usd, other.cost_usd);
        self.tool_errors += other.tool_errors;
        self.permission_denials += other.permission_denials;
        self.killed_work += other.killed_work;
        self.repeated_tool_calls += other.repeated_tool_calls;
        if let Some(flags) = &other.destructive_commands {
            self.destructive_commands
                .get_or_insert_default()
                .extend(flags.iter().cloned());
        }
    }
}

fn sum<T: Default + std::ops::AddAssign>(total: &mut Option<T>, value: Option<T>) {
    if let Some(value) = value {
        *total.get_or_insert_default() += value;
    }
}

impl ModelTokens {
    fn of(model: Option<String>, usage: &Usage, excludes_cache: bool) -> Self {
        Self {
            model,
            input_tokens: usage.input_including_cache(excludes_cache),
            output_tokens: usage.output,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
        }
    }

    fn add(&mut self, other: &Self) {
        sum(&mut self.input_tokens, other.input_tokens);
        sum(&mut self.output_tokens, other.output_tokens);
        sum(&mut self.cache_read_tokens, other.cache_read_tokens);
        sum(&mut self.cache_write_tokens, other.cache_write_tokens);
    }
}

fn text(attrs: &[KeyValue], key: &str) -> Option<String> {
    attrs.iter().find_map(|attr| match &attr.value {
        Some(AnyValue::String(value)) if attr.key == key => Some(value.clone()),
        _ => None,
    })
}

/// Read normalized facts, including retained amendments. Raw Harness bodies are
/// never reparsed. A missing or unreadable trace is missing evidence to callers.
pub(crate) fn read(log: &Path) -> anyhow::Result<Option<SessionSignals>> {
    let path = super::session::path(log);
    // A failed recorder can leave a FIFO or device behind. Reading it while
    // ending a Run must not block; only regular files contain retained traces.
    if !path.is_file() {
        return Ok(None);
    }
    let Some(requests) = super::session::read_requests::<Logs>(&path)? else {
        return Ok(None);
    };
    let mut harness = None;
    let mut harness_version = None;
    let mut directory = None;
    let mut events = Vec::new();
    let mut work_aliases = BTreeMap::new();
    for request in requests {
        for resource in request.resource_logs {
            if let Some(resource) = resource.resource {
                harness = text(&resource.attributes, "thirdshift.harness").or(harness);
                harness_version =
                    text(&resource.attributes, "thirdshift.harness.version").or(harness_version);
                directory =
                    text(&resource.attributes, "thirdshift.harness.directory").or(directory);
            }
            for record in resource
                .scope_logs
                .into_iter()
                .flat_map(|scope| scope.log_records)
            {
                if let Some(json) = text(&record.attributes, "thirdshift.events") {
                    let mut batch = Vec::new();
                    for value in serde_json::from_str::<Vec<serde_json::Value>>(&json)? {
                        batch.push(serde_json::from_value::<Event>(value)?);
                    }
                    work_aliases.extend(ended_work_alias(&batch));
                    events.extend(batch);
                }
            }
        }
    }
    let excludes_cache = harness
        .as_deref()
        .and_then(crate::harness::Harness::named)
        .is_some_and(crate::harness::Harness::input_excludes_cache);
    let mut turns = BTreeMap::new();
    let mut session_usage = None;
    let mut calls = BTreeMap::new();
    let mut reported_models = BTreeMap::new();
    let mut killed = HashSet::new();
    for event in events {
        if let Some(call) = crate::session::trace::Call::of(&event) {
            let key = call.key();
            // An end refines its start, and a retained end amends the same call.
            if call.completion.is_some() || !calls.contains_key(&key) {
                calls.insert(key, event.clone());
            }
        }
        match event {
            Event::TurnEnded { id, usage, .. } => {
                turns.insert(id, usage);
            }
            Event::Usage {
                scope: UsageScope::Model,
                model,
                usage,
                cost_usd,
            } => {
                reported_models.insert(model, (usage, cost_usd));
            }
            Event::Usage {
                scope: UsageScope::Session,
                model,
                usage,
                cost_usd,
            } => {
                session_usage = Some((model, usage, cost_usd));
            }
            Event::WorkEnded {
                id,
                outcome: Outcome::Killed,
            } => {
                killed.insert(work_aliases.get(&id).cloned().unwrap_or(id));
            }
            _ => {}
        }
    }
    let mut signals = Signals::default();
    for event in calls.values() {
        if let Event::ModelCallEnded {
            model,
            usage,
            cost_usd,
            ..
        } = event
        {
            signals.add(&Signals {
                tokens_by_model: (usage != &Usage::default())
                    .then(|| ModelTokens::of(model.clone(), usage, excludes_cache))
                    .into_iter()
                    .collect(),
                cost_usd: *cost_usd,
                ..Default::default()
            });
        }
    }
    // Per-Model totals replace per-call sums for those Models; they are not
    // additional calls. A session total likewise never adds to call usage.
    for (model, (usage, _)) in &reported_models {
        let tokens = ModelTokens::of(model.clone(), usage, excludes_cache);
        if let Some(old) = signals
            .tokens_by_model
            .iter_mut()
            .find(|t| &t.model == model)
        {
            *old = tokens;
        } else {
            signals.tokens_by_model.push(tokens);
        }
    }
    if !reported_models.is_empty() {
        signals.cost_usd = None;
        for (_, cost) in reported_models.values() {
            sum(&mut signals.cost_usd, *cost);
        }
    }
    if signals.tokens_by_model.is_empty() {
        let mut tokens = ModelTokens::default();
        if let Some((model, usage, _)) = &session_usage {
            tokens = ModelTokens::of(model.clone(), usage, excludes_cache);
        } else {
            for usage in turns.values() {
                tokens.add(&ModelTokens::of(None, usage, excludes_cache));
            }
        }
        if tokens.input_tokens.is_some()
            || tokens.output_tokens.is_some()
            || tokens.cache_read_tokens.is_some()
            || tokens.cache_write_tokens.is_some()
        {
            signals.tokens_by_model.push(tokens);
        }
    }
    if let Some((_, _, Some(cost))) = session_usage {
        signals.cost_usd = Some(cost);
    }
    signals.destructive_commands = Some(Vec::new());
    let mut inputs = HashSet::new();
    for event in calls.values() {
        if let Event::ToolCallEnded {
            id,
            outcome: Outcome::Killed,
            ..
        }
        | Event::SubagentEnded {
            id,
            outcome: Outcome::Killed,
            ..
        } = event
        {
            // A tool, agent and WorkEnded can describe the same killed work.
            killed.insert(id.clone());
        }
        match event {
            Event::ToolCallStarted { tool, input, .. }
            | Event::ToolCallEnded { tool, input, .. } => {
                signals
                    .destructive_commands
                    .as_mut()
                    .unwrap()
                    .extend(destructive::check(
                        tool,
                        input,
                        directory.as_deref().map(Path::new),
                    ));
                if !inputs.insert(serde_json::to_string(&(tool, input))?) {
                    signals.repeated_tool_calls += 1;
                }
                if let Event::ToolCallEnded { outcome, .. } = event {
                    signals.tool_errors += u64::from(*outcome == Outcome::Failed);
                    signals.permission_denials += u64::from(*outcome == Outcome::Denied);
                }
            }
            _ => {}
        }
    }
    signals.killed_work += killed.len() as u64;
    Ok(Some(SessionSignals {
        harness,
        harness_version,
        signals,
    }))
}

/// A task notification ends its owning agent last, after inner work. Its
/// WorkEnded can name a task ID rather than that agent's call ID. The normalized
/// facts share one record even in converted traces, so retain that association
/// before flattening records or counting retained amendments.
fn ended_work_alias(events: &[Event]) -> Option<(String, String)> {
    let ended: HashSet<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::ToolCallEnded { id, .. } | Event::SubagentEnded { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut unmatched = events.iter().filter_map(|event| match event {
        Event::WorkEnded {
            id,
            outcome: Outcome::Killed,
        } if !ended.contains(id.as_str()) => Some(id),
        _ => None,
    });
    let task = unmatched.next()?;
    if unmatched.next().is_some() {
        return None;
    }
    let agent = events.iter().rev().find_map(|event| match event {
        Event::SubagentEnded {
            id,
            outcome: Outcome::Killed,
            ..
        } => Some(id),
        _ => None,
    })?;
    Some((task.clone(), agent.clone()))
}
