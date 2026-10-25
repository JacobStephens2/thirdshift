//! Usage comes from retained Session spans, never from requested Models or
//! prices. Model totals supersede calls; session cost supersedes model costs.
use std::collections::BTreeMap;
use std::io::BufRead;

use anyhow::Result;
use serde::Serialize;

use super::Arm as TrialArm;
use crate::otlp_jsonl::{AnyValue, KeyValue, Reader, Span, Spans};

#[derive(Default, Serialize)]
pub(crate) struct Arms {
    pub baseline: Arm,
    pub candidate: Arm,
}

#[derive(Default, Serialize)]
pub(crate) struct Arm {
    models: BTreeMap<String, Measurement>,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Copy, Default, Serialize)]
struct Measurement {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

impl Arms {
    pub(super) fn include(&mut self, arm: TrialArm, spans: impl BufRead) -> Result<()> {
        let arm = match arm {
            TrialArm::Baseline => &mut self.baseline,
            TrialArm::Candidate => &mut self.candidate,
        };
        arm.include(Arm::session(spans)?);
        Ok(())
    }

    pub fn baseline(&self) -> String {
        self.baseline.to_string()
    }
    pub fn candidate(&self) -> String {
        self.candidate.to_string()
    }
}

impl Arm {
    pub fn tokens(&self) -> Option<u64> {
        let mut total = None;
        for measurement in self.models.values() {
            for count in [measurement.input_tokens, measurement.output_tokens]
                .into_iter()
                .flatten()
            {
                total = Some(total.unwrap_or(0_u64).saturating_add(count));
            }
        }
        total
    }
    fn session(reader: impl BufRead) -> Result<Self> {
        let mut spans = BTreeMap::new();
        let mut excludes_cache = false;
        for request in Reader::<_, Spans>::new(reader) {
            for resource in request?.resource_spans {
                if let Some(resource) = resource.resource {
                    excludes_cache |= matches!(
                        text(&resource.attributes, "thirdshift.harness"),
                        Some("claude" | "grok")
                    );
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
        let session = spans
            .values()
            .find(|span| span.name.starts_with("invoke_agent "));
        let mut arm = Self::default();
        // Call spans already normalize inclusive input. Turn totals have no
        // reported Model in Codex exec and remain explicitly unknown.
        for span in spans.values().filter(|span| {
            text(&span.attributes, "gen_ai.operation.name") == Some("chat")
                || span.name.starts_with("turn ")
        }) {
            let measurement = Measurement::span(span);
            if measurement.reported() {
                let model = text(&span.attributes, "gen_ai.response.model").unwrap_or("unknown");
                arm.models
                    .entry(model.into())
                    .or_default()
                    .include(measurement);
            }
        }
        if let Some(session) = session {
            let total = Measurement::span(session);
            let model = text(&session.attributes, "thirdshift.model").unwrap_or("unknown");
            // A Session total can refine partial calls when all measured usage
            // belongs to that Model. Keep multi-Model totals unattributed.
            if total.reported()
                && arm.models.len() == 1
                && let Some(measurement) = arm.models.get_mut(model)
            {
                measurement.prefer_total(
                    Measurement {
                        cost_usd: None,
                        ..total
                    },
                    false,
                );
            }
            if let Some(models) = text(&session.attributes, "thirdshift.model_usage") {
                let models: BTreeMap<String, serde_json::Value> = serde_json::from_str(models)?;
                for (model, event) in models {
                    if event["type"] == "usage" {
                        let usage = &event["usage"];
                        let measurement = Measurement {
                            input_tokens: usage["input"].as_u64(),
                            output_tokens: usage["output"].as_u64(),
                            cache_read_tokens: usage["cache_read"].as_u64(),
                            cache_write_tokens: usage["cache_write"].as_u64(),
                            cost_usd: event["cost_usd"].as_f64(),
                        };
                        if measurement.reported() {
                            arm.models
                                .entry(if model.is_empty() {
                                    "unknown".into()
                                } else {
                                    model
                                })
                                .or_default()
                                .prefer_total(measurement, excludes_cache);
                        }
                    }
                }
            }
            if arm.models.is_empty() && total.reported() {
                arm.models.insert(
                    model.into(),
                    Measurement {
                        cost_usd: None,
                        ..total
                    },
                );
            }
            arm.cost_usd = number(&session.attributes, "thirdshift.cost.usd");
        }
        if arm.cost_usd.is_none() {
            for model in arm.models.values() {
                add_cost(&mut arm.cost_usd, model.cost_usd);
            }
        }
        Ok(arm)
    }

    fn include(&mut self, other: Self) {
        for (model, measurement) in other.models {
            self.models.entry(model).or_default().include(measurement);
        }
        add_cost(&mut self.cost_usd, other.cost_usd);
    }
}

impl std::fmt::Display for Arm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (model, measurement) in &self.models {
            write!(
                f,
                " · {model}: {} input, {} output tokens",
                measurement
                    .input_tokens
                    .map_or("unknown".into(), |v| v.to_string()),
                measurement
                    .output_tokens
                    .map_or("unknown".into(), |v| v.to_string())
            )?;
        }
        if let Some(cost) = self.cost_usd {
            write!(f, " · ${cost:.6}")?;
        }
        Ok(())
    }
}

impl Measurement {
    // A Model total supersedes only the fields the Harness actually reported.
    fn prefer_total(&mut self, total: Self, input_excludes_cache: bool) {
        // Calls have inclusive input already. Recover the uncached count so
        // partial Model totals can replace input or cache independently.
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

    fn span(span: &Span) -> Self {
        Self {
            input_tokens: count(&span.attributes, "gen_ai.usage.input_tokens"),
            output_tokens: count(&span.attributes, "gen_ai.usage.output_tokens"),
            cache_read_tokens: count(&span.attributes, "thirdshift.usage.cache_read_tokens"),
            cache_write_tokens: count(&span.attributes, "thirdshift.usage.cache_write_tokens"),
            cost_usd: number(&span.attributes, "thirdshift.cost.usd"),
        }
    }

    fn reported(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
            || self.cache_write_tokens.is_some()
            || self.cost_usd.is_some()
    }

    fn include(&mut self, other: Self) {
        for (total, value) in [
            (&mut self.input_tokens, other.input_tokens),
            (&mut self.output_tokens, other.output_tokens),
            (&mut self.cache_read_tokens, other.cache_read_tokens),
            (&mut self.cache_write_tokens, other.cache_write_tokens),
        ] {
            if let Some(value) = value {
                *total = Some(total.unwrap_or(0).saturating_add(value));
            }
        }
        add_cost(&mut self.cost_usd, other.cost_usd);
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

fn text<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
    match value(attrs, key)? {
        AnyValue::String(s) => Some(s),
        _ => None,
    }
}

fn count(attrs: &[KeyValue], key: &str) -> Option<u64> {
    match value(attrs, key)? {
        AnyValue::Int(n) => (*n).try_into().ok(),
        AnyValue::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn number(attrs: &[KeyValue], key: &str) -> Option<f64> {
    match value(attrs, key)? {
        AnyValue::Double(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}
