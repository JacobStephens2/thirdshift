//! Session usage from normalized facts or retained spans. Acquisition and
//! consumer grouping stay with callers; precedence and cache conventions live here.
use std::collections::BTreeMap;
use std::io::BufRead;

use anyhow::Result;
use serde::Serialize;

use super::trace::{Calls, event_attributes};
use crate::harness::Harness;
use crate::harness::interpretation::event::{Event, Usage, UsageScope};
use crate::logs::trace::SpanKind;
use crate::otlp_jsonl::{AnyValue, Id, KeyValue, Reader, Spans};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Summary {
    // First-observation order keeps the metrics projection stable.
    pub models: Vec<Model>,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Model {
    pub model: Option<String>,
    pub measurement: Measurement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub(crate) struct Measurement {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

impl Summary {
    /// Add separate Sessions only after resolving their cumulative totals.
    pub fn add(&mut self, other: &Self) {
        for model in &other.models {
            self.measurement(model.model.clone())
                .include(model.measurement);
        }
        add_cost(&mut self.cost_usd, other.cost_usd);
    }

    pub fn tokens(&self) -> Option<u64> {
        let mut total = None;
        for model in &self.models {
            add_count(&mut total, model.measurement.input_tokens);
            add_count(&mut total, model.measurement.output_tokens);
        }
        total
    }

    fn measurement(&mut self, model: Option<String>) -> &mut Measurement {
        let index = self
            .models
            .iter()
            .position(|m| m.model == model)
            .unwrap_or_else(|| {
                self.models.push(Model {
                    model,
                    measurement: Measurement::default(),
                });
                self.models.len() - 1
            });
        &mut self.models[index].measurement
    }
}

/// The cache convention is observed Harness metadata, never requested Model setup.
/// Calls owns scoped identity, repeated completions and present-field amendments.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only usage events measure a Model or the Session; calls take the other events above"
)]
pub(crate) fn from_events(events: &[Event], input_excludes_cache: bool) -> Summary {
    let mut calls = Calls::new(Id([0; 8]), input_excludes_cache, []);
    let mut evidence = Evidence::default();
    let mut session_attributes = Vec::new();
    for event in events {
        calls.apply(event, None);
        match event {
            Event::Usage {
                scope: UsageScope::Model,
                model,
                usage,
                cost_usd,
            } => {
                evidence
                    .models
                    .insert(model.clone(), Measurement::raw(usage, *cost_usd));
            }
            Event::Usage {
                scope: UsageScope::Session,
                ..
            } => {
                // Retained Session attributes keep present fields, including a
                // previously observed Model, through sparse Session observations.
                for attr in event_attributes(event, input_excludes_cache) {
                    super::trace::set(&mut session_attributes, attr);
                }
            }
            _ => {}
        }
    }
    for call in calls.calls() {
        evidence.call(call.kind, &call.attributes);
    }
    evidence.session = Measurement::attributes(&session_attributes);
    evidence.session_model = text(&session_attributes, "thirdshift.model").map(str::to_owned);
    evidence.finish(input_excludes_cache)
}

/// Read already selected evidence once, retaining the latest span for each ID.
/// This operation does no filesystem discovery or writes.
pub(crate) fn from_spans(reader: impl BufRead) -> Result<Summary> {
    let mut spans = BTreeMap::new();
    let mut excludes_cache = false;
    for request in Reader::<_, Spans>::new(reader) {
        for resource in request?.resource_spans {
            if let Some(resource) = resource.resource {
                excludes_cache |= text(&resource.attributes, "thirdshift.harness")
                    .and_then(Harness::named)
                    .is_some_and(Harness::input_excludes_cache);
            }
            for span in resource
                .scope_spans
                .into_iter()
                .flat_map(|scope| scope.spans)
            {
                if let Some(id) = span.span_id {
                    spans.insert(id.0, span);
                }
            }
        }
    }
    let mut evidence = Evidence::default();
    for span in spans.values() {
        if text(&span.attributes, "gen_ai.operation.name") == Some("chat") {
            evidence.call(SpanKind::Model, &span.attributes);
        } else if span.name.starts_with("turn ") {
            evidence.call(SpanKind::Turn, &span.attributes);
        }
    }
    if let Some(session) = spans
        .values()
        .find(|span| span.name.starts_with("invoke_agent "))
    {
        evidence.session = Measurement::attributes(&session.attributes);
        evidence.session_model = text(&session.attributes, "thirdshift.model").map(str::to_owned);
        if let Some(models) = text(&session.attributes, "thirdshift.model_usage") {
            let models: BTreeMap<String, serde_json::Value> = serde_json::from_str(models)?;
            for (model, event) in models {
                if event["type"] == "usage" {
                    let usage = &event["usage"];
                    evidence.models.insert(
                        (!model.is_empty()).then_some(model),
                        Measurement::raw(
                            &Usage {
                                input: usage["input"].as_u64(),
                                output: usage["output"].as_u64(),
                                cache_read: usage["cache_read"].as_u64(),
                                cache_write: usage["cache_write"].as_u64(),
                            },
                            event["cost_usd"].as_f64(),
                        ),
                    );
                }
            }
        }
    }
    Ok(evidence.finish(excludes_cache))
}

#[derive(Default)]
struct Evidence {
    calls: Summary,
    turns: Measurement,
    models: BTreeMap<Option<String>, Measurement>,
    session: Measurement,
    session_model: Option<String>,
}

impl Evidence {
    fn call(&mut self, kind: SpanKind, attrs: &[KeyValue]) {
        let measurement = Measurement::attributes(attrs);
        match kind {
            SpanKind::Model if measurement.reported() => {
                self.calls
                    .measurement(text(attrs, "gen_ai.response.model").map(str::to_owned))
                    .include(measurement);
            }
            SpanKind::Turn => self.turns.include(measurement),
            SpanKind::Cmd
            | SpanKind::Run
            | SpanKind::Stage
            | SpanKind::Sess
            | SpanKind::Model
            | SpanKind::Tool
            | SpanKind::Agent => {}
        }
    }

    fn finish(mut self, input_excludes_cache: bool) -> Summary {
        for (model, total) in self.models {
            if total.reported() {
                self.calls
                    .measurement(model)
                    .prefer_total(total, input_excludes_cache);
            }
        }
        let measured: Vec<_> = self
            .calls
            .models
            .iter()
            .enumerate()
            .filter(|(_, m)| m.measurement.tokens_reported())
            .map(|(i, _)| i)
            .collect();
        let session_tokens = Measurement {
            cost_usd: None,
            ..self.session
        };
        if self.session.tokens_reported() {
            if measured.is_empty() {
                self.calls
                    .measurement(self.session_model)
                    .prefer_total(session_tokens, false);
            } else if measured.len() == 1
                && self.calls.models[measured[0]].model == self.session_model
            {
                self.calls.models[measured[0]]
                    .measurement
                    .prefer_total(session_tokens, false);
            }
        } else if measured.is_empty() && self.turns.tokens_reported() {
            self.calls.measurement(None).include(Measurement {
                cost_usd: None,
                ..self.turns
            });
        }
        // Cost is independent of token coverage. Omitted totals cannot erase it.
        self.calls.cost_usd = self.session.cost_usd;
        if self.calls.cost_usd.is_none() {
            for model in &self.calls.models {
                add_cost(&mut self.calls.cost_usd, model.measurement.cost_usd);
            }
        }
        self.calls
    }
}

impl Measurement {
    pub fn tokens_reported(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_write_tokens.is_some()
    }

    fn reported(&self) -> bool {
        self.tokens_reported() || self.cost_usd.is_some()
    }

    fn raw(usage: &Usage, cost_usd: Option<f64>) -> Self {
        Self {
            input_tokens: usage.input,
            output_tokens: usage.output,
            cache_read_tokens: usage.cache_read,
            cache_write_tokens: usage.cache_write,
            cost_usd,
        }
    }

    fn attributes(attrs: &[KeyValue]) -> Self {
        Self {
            input_tokens: count(attrs, "gen_ai.usage.input_tokens"),
            output_tokens: count(attrs, "gen_ai.usage.output_tokens"),
            cache_read_tokens: count(attrs, "thirdshift.usage.cache_read_tokens"),
            cache_write_tokens: count(attrs, "thirdshift.usage.cache_write_tokens"),
            cost_usd: match value(attrs, "thirdshift.cost.usd") {
                Some(AnyValue::Double(n)) if n.is_finite() => Some(*n),
                _ => None,
            },
        }
    }

    fn prefer_total(&mut self, total: Self, input_excludes_cache: bool) {
        // Calls and Session attributes are inclusive. Only raw Model totals
        // need uncached input recovered before replacing input/cache separately.
        let input = total.input_tokens.or_else(|| {
            self.input_tokens.map(|input| {
                if input_excludes_cache {
                    input
                        .saturating_sub(self.cache_read_tokens.unwrap_or(0))
                        .saturating_sub(self.cache_write_tokens.unwrap_or(0))
                } else {
                    input
                }
            })
        });
        self.output_tokens = total.output_tokens.or(self.output_tokens);
        self.cache_read_tokens = total.cache_read_tokens.or(self.cache_read_tokens);
        self.cache_write_tokens = total.cache_write_tokens.or(self.cache_write_tokens);
        self.cost_usd = total.cost_usd.or(self.cost_usd);
        self.input_tokens = input.map(|input| {
            if input_excludes_cache {
                input
                    .saturating_add(self.cache_read_tokens.unwrap_or(0))
                    .saturating_add(self.cache_write_tokens.unwrap_or(0))
            } else {
                input
            }
        });
    }

    fn include(&mut self, other: Self) {
        add_count(&mut self.input_tokens, other.input_tokens);
        add_count(&mut self.output_tokens, other.output_tokens);
        add_count(&mut self.cache_read_tokens, other.cache_read_tokens);
        add_count(&mut self.cache_write_tokens, other.cache_write_tokens);
        add_cost(&mut self.cost_usd, other.cost_usd);
    }
}

fn add_count(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0).saturating_add(value));
    }
}

fn add_cost(total: &mut Option<f64>, value: Option<f64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or(0.0) + value);
    }
}

fn value<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a AnyValue> {
    attrs.iter().find(|attr| attr.key == key)?.value.as_ref()
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "an OTLP attribute that isn't a string has no text"
)]
fn text<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
    match value(attrs, key)? {
        AnyValue::String(s) => Some(s),
        _ => None,
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "an OTLP attribute that is neither an integer nor a string has no count"
)]
fn count(attrs: &[KeyValue], key: &str) -> Option<u64> {
    match value(attrs, key)? {
        AnyValue::Int(n) => (*n).try_into().ok(),
        AnyValue::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
