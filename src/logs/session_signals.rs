//! Measurements from one normalized Session trace, without Harness price tables.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::harness::interpretation::event::{Event, Outcome};
use crate::otlp_jsonl::{AnyValue, KeyValue, Logs};
use crate::session::usage::{self, Measurement, Model};

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

#[derive(Clone, Default, Serialize, Deserialize)]
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
        let mut usage = self.usage();
        usage.add(&other.usage());
        self.set_usage(usage);
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

    fn usage(&self) -> usage::Summary {
        usage::Summary {
            models: self
                .tokens_by_model
                .iter()
                .map(|tokens| Model {
                    model: tokens.model.clone(),
                    measurement: Measurement {
                        input_tokens: tokens.input_tokens,
                        output_tokens: tokens.output_tokens,
                        cache_read_tokens: tokens.cache_read_tokens,
                        cache_write_tokens: tokens.cache_write_tokens,
                        cost_usd: None,
                    },
                })
                .collect(),
            cost_usd: self.cost_usd,
        }
    }

    fn set_usage(&mut self, summary: usage::Summary) {
        self.cost_usd = summary.cost_usd;
        self.tokens_by_model = summary
            .models
            .into_iter()
            .filter(|model| model.measurement.tokens_reported())
            .map(|model| ModelTokens {
                model: model.model,
                input_tokens: model.measurement.input_tokens,
                output_tokens: model.measurement.output_tokens,
                cache_read_tokens: model.measurement.cache_read_tokens,
                cache_write_tokens: model.measurement.cache_write_tokens,
            })
            .collect();
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
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only tool calls carry a command to check; other Session events give no signal here"
)]
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
    let mut signals = Signals::default();
    signals.set_usage(usage::from_events(&events, excludes_cache));
    let mut lifecycle =
        crate::session::trace::Calls::new(crate::otlp_jsonl::Id([0; 8]), excludes_cache, []);
    let mut calls = BTreeMap::new();
    let mut killed = HashSet::new();
    for event in events {
        if let Some(applied) = lifecycle.apply(&event, None) {
            let key = serde_json::to_string(&(
                applied.call.kind,
                &applied.call.parent_id,
                &applied.call.id,
            ))?;
            // An end refines its start, and a retained end amends the same call.
            if matches!(
                event,
                Event::ModelCallEnded { .. }
                    | Event::ToolCallEnded { .. }
                    | Event::TurnEnded { .. }
                    | Event::SubagentEnded { .. }
            ) || !calls.contains_key(&key)
            {
                calls.insert(key, event.clone());
            }
        }
        if let Event::WorkEnded {
            id,
            outcome: Outcome::Killed,
        } = event
        {
            killed.insert(work_aliases.get(&id).cloned().unwrap_or(id));
        }
    }
    let mut destructive_commands = Vec::new();
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
                destructive_commands.extend(destructive::check(
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
    // Older log resources did not retain the Worktree directory. Keep known
    // matches, but an unperformed outside-Worktree check cannot imply zero.
    signals.destructive_commands =
        (directory.is_some() || !destructive_commands.is_empty()).then_some(destructive_commands);
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
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "each search picks one kind of ending out of the Session's events and skips the rest"
)]
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
