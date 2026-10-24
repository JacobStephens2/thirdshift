//! Merge ended OTLP spans with normalized start records for live Sessions.
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::{Span, duration, hex};
use crate::harness::interpretation::event::{Event, Outcome, UsageScope};
use crate::logs::trace::{Context, SpanKind, id};
use crate::otlp_jsonl::{AnyValue, KeyValue, Logs, Spans};
use crate::session::trace::{Calls, Retained, event_attributes};

pub(super) fn read(path: &Path, context: &Context, template: &Span) -> Result<Option<Vec<Span>>> {
    let original = crate::logs::session::original(path);
    let requests =
        crate::logs::session::read_requests::<Spans>(&original.with_extension("spans.jsonl"))?;
    let logs = crate::logs::session::read_requests::<Logs>(&original.with_extension("logs.jsonl"))?;
    if requests.is_none() && logs.is_none() {
        return Ok(None);
    }
    let mut completed = Vec::new();
    let mut resource_attrs = BTreeMap::new();
    for request in requests.into_iter().flatten() {
        for resource in request.resource_spans {
            let attrs = resource
                .resource
                .map(|resource| attributes(&resource.attributes))
                .unwrap_or_default();
            resource_attrs.extend(attrs.clone());
            for raw in resource
                .scope_spans
                .into_iter()
                .flat_map(|scope| scope.spans)
            {
                let mut attrs = attrs.clone();
                attrs.extend(attributes(&raw.attributes));
                let mut span = template.clone();
                span.id = hex(raw.span_id.unwrap_or_else(|| {
                    id(&[
                        &context.run.0,
                        original.as_os_str().as_encoded_bytes(),
                        raw.name.as_bytes(),
                        &raw.start_time_unix_nano.to_be_bytes(),
                    ])
                }));
                span.parent_id = raw.parent_span_id.map(hex);
                span.kind = kind(&attrs);
                span.name = raw.name;
                span.start = millis(raw.start_time_unix_nano);
                span.end = millis(raw.end_time_unix_nano);
                span.duration_ms = raw
                    .end_time_unix_nano
                    .checked_sub(raw.start_time_unix_nano)
                    .filter(|_| raw.start_time_unix_nano != 0 && raw.end_time_unix_nano != 0)
                    .map(|nanos| nanos as f64 / 1_000_000.0);
                span.error = raw
                    .status
                    .as_ref()
                    .filter(|status| status.code == 2)
                    .map(|status| status.message.clone());
                span.status = if span.error.is_some() {
                    "error"
                } else if span.end.is_some() {
                    "ok"
                } else if template.status == "running" {
                    "running"
                } else {
                    "unknown"
                }
                .into();
                apply_attributes(&mut span, attrs);
                completed.push(span);
            }
        }
    }
    let session_id = completed
        .iter()
        .find(|span| span.kind == SpanKind::Sess)
        .map(|span| span.id.clone())
        .unwrap_or_else(|| {
            hex(id(&[
                &context.trace.0,
                b"session",
                original.as_os_str().as_encoded_bytes(),
            ]))
        });
    let session_bytes: crate::otlp_jsonl::SpanId =
        serde_json::from_value(Value::String(session_id.clone()))?;
    let mut session = completed
        .iter()
        .find(|span| span.kind == SpanKind::Sess)
        .cloned()
        .unwrap_or_else(|| {
            let mut session = template.clone();
            session.id = session_id.clone();
            session.name = "Session".into();
            session.error = None;
            session
        });
    // A shared Session belongs to the first stage naming it, even when an old
    // trace points at the later pull-request stage that reused that Session.
    session.parent_id.clone_from(&template.parent_id);
    // Completed spans retain the identity of their call, including migrated
    // or custom IDs, so replayed start records land on the same span.
    let retained = completed.iter().filter_map(|span| {
        Some(Retained {
            kind: span.kind,
            id: span
                .attributes
                .get("gen_ai.tool.call.id")
                .or_else(|| span.attributes.get("thirdshift.id"))?
                .as_str()?,
            parent: span
                .attributes
                .get("thirdshift.parent_id")
                .and_then(Value::as_str),
            span: serde_json::from_value(Value::String(span.id.clone())).ok()?,
        })
    });
    let harness = resource_attrs
        .get("thirdshift.harness")
        .cloned()
        .or_else(|| {
            logs.iter()
                .flatten()
                .flat_map(|request| &request.resource_logs)
                .filter_map(|resource| resource.resource.as_ref())
                .find_map(|resource| attributes(&resource.attributes).remove("thirdshift.harness"))
        });
    let excludes_cache = matches!(
        harness.as_ref().and_then(Value::as_str),
        Some("claude" | "grok")
    );
    let mut replay = Calls::new(session_bytes, excludes_cache, retained);
    // Evidence the reader adds to calls: resource attributes and estimated times.
    let mut evidence = BTreeMap::<String, BTreeMap<String, Value>>::new();
    let mut first_record = None::<f64>;
    for request in logs.into_iter().flatten() {
        for resource in request.resource_logs {
            resource_attrs.extend(
                resource
                    .resource
                    .map(|resource| attributes(&resource.attributes))
                    .unwrap_or_default(),
            );
            for record in resource
                .scope_logs
                .into_iter()
                .flat_map(|scope| scope.log_records)
            {
                let attrs = attributes(&record.attributes);
                for key in [
                    "thirdshift.session.prompt",
                    "thirdshift.session.start_time_unix_nano",
                ] {
                    if let Some(value) = attrs.get(key) {
                        session.attributes.insert(key.into(), value.clone());
                    }
                }
                // Zero means an unknown clock.
                let time = [record.time_unix_nano, record.observed_time_unix_nano]
                    .into_iter()
                    .find(|time| *time != 0);
                let start = time.and_then(millis);
                first_record = match (first_record, start) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                let Some(events) = attrs.get("thirdshift.events").and_then(Value::as_str) else {
                    continue;
                };
                let events: Vec<Value> =
                    serde_json::from_str(events).context("invalid Session-trace events")?;
                for event in events {
                    let Ok(event) = serde_json::from_value::<Event>(event) else {
                        continue;
                    };
                    let Some(applied) = replay.apply(&event, time) else {
                        if matches!(
                            event,
                            Event::Usage {
                                scope: UsageScope::Session,
                                ..
                            }
                        ) {
                            session
                                .attributes
                                .extend(attributes(&event_attributes(&event, excludes_cache)));
                        }
                        continue;
                    };
                    let call_evidence = evidence.entry(hex(applied.call.span)).or_default();
                    call_evidence.extend(resource_attrs.clone());
                    if attrs.get("thirdshift.time.estimated") == Some(&Value::Bool(true)) {
                        call_evidence.insert("thirdshift.time.estimated".into(), Value::Bool(true));
                    }
                }
            }
        }
    }
    let mut calls: BTreeMap<_, _> = replay
        .calls()
        .map(|call| {
            let mut span = template.clone();
            span.id = hex(call.span);
            span.parent_id = Some(hex(call.parent_span));
            span.kind = call.kind;
            span.name = format!("{} {}", call.kind, call.id);
            span.start = call.started.and_then(millis);
            span.end = call.ended.and_then(millis);
            span.duration_ms = duration(span.start, span.end);
            // A later success keeps the last failure as its diagnostic.
            span.error = call.failure.map(|outcome| format!("{outcome:?}"));
            span.status = match call.outcome {
                Some(Outcome::Succeeded) => "ok",
                Some(_) => "error",
                None if template.status == "running" => "running",
                None => "unknown",
            }
            .into();
            let mut attrs = evidence.remove(&span.id).unwrap_or_default();
            attrs.extend(attributes(&call.attributes));
            apply_attributes(&mut span, attrs);
            (span.id.clone(), span)
        })
        .collect();
    if !completed.iter().any(|span| span.kind == SpanKind::Sess) {
        session.start = session
            .attributes
            .get("thirdshift.session.start_time_unix_nano")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .and_then(millis)
            .or(first_record)
            .or(session.start);
        session.duration_ms = duration(session.start, session.end);
        resource_attrs.extend(session.attributes.clone());
        apply_attributes(&mut session, resource_attrs);
    }
    for mut span in completed
        .into_iter()
        .filter(|span| span.kind != SpanKind::Sess)
    {
        if span.parent_id.is_none() {
            span.parent_id = Some(session_id.clone());
        }
        calls.insert(span.id.clone(), span);
    }
    let mut spans = vec![session];
    for mut span in calls.into_values() {
        if span.session_kind.is_none() {
            span.session_kind.clone_from(&spans[0].session_kind);
        }
        // Broken or missing causal references should not hide retained evidence.
        if span
            .parent_id
            .as_ref()
            .is_none_or(|parent| parent == &span.id)
        {
            span.parent_id = Some(session_id.clone());
        }
        spans.push(span);
    }
    let ids: std::collections::HashSet<_> = spans.iter().map(|span| span.id.clone()).collect();
    for span in spans.iter_mut().skip(1) {
        if span
            .parent_id
            .as_ref()
            .is_none_or(|parent| !ids.contains(parent))
        {
            span.parent_id = Some(session_id.clone());
        }
    }
    Ok(Some(spans))
}

fn kind(attrs: &BTreeMap<String, Value>) -> SpanKind {
    match attrs.get("gen_ai.operation.name").and_then(Value::as_str) {
        Some("invoke_agent") => SpanKind::Sess,
        Some("execute_tool") => SpanKind::Tool,
        Some("chat") => SpanKind::Model,
        _ => match attrs
            .get("thirdshift.type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            value if value.starts_with("subagent_") => SpanKind::Agent,
            value if value.starts_with("turn_") => SpanKind::Turn,
            _ => SpanKind::Model,
        },
    }
}

fn apply_attributes(span: &mut Span, attrs: BTreeMap<String, Value>) {
    span.estimated = attrs
        .get("thirdshift.time.estimated")
        .is_some_and(|value| value == &Value::Bool(true) || value == "true");
    span.session_kind = text(&attrs, "thirdshift.session.kind").or(span.session_kind.clone());
    span.input = text(&attrs, "gen_ai.tool.call.arguments")
        .or_else(|| text(&attrs, "thirdshift.session.prompt"))
        .or_else(|| text(&attrs, "thirdshift.description"));
    span.output =
        text(&attrs, "gen_ai.tool.call.result").or_else(|| text(&attrs, "thirdshift.result"));
    if span.kind == SpanKind::Tool
        && let Some(tool) = text(&attrs, "gen_ai.tool.name")
    {
        span.name = format!(
            "{tool}{}",
            span.input
                .as_ref()
                .map(|input| format!(" {input}"))
                .unwrap_or_default()
        );
    }
    span.attributes = attrs;
}

fn text(attrs: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    attrs
        .get(key)
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
}

fn millis(nanos: u64) -> Option<f64> {
    (nanos != 0).then_some((nanos / 1_000_000) as f64 + (nanos % 1_000_000) as f64 / 1_000_000.0)
}

fn attributes(attrs: &[KeyValue]) -> BTreeMap<String, Value> {
    attrs
        .iter()
        .filter_map(|attr| {
            attr.value
                .as_ref()
                .map(|value| (attr.key.clone(), value_json(value)))
        })
        .collect()
}

fn value_json(value: &AnyValue) -> Value {
    match value {
        AnyValue::Empty => Value::Null,
        AnyValue::String(value) => Value::String(value.clone()),
        AnyValue::Bool(value) => Value::Bool(*value),
        AnyValue::Int(value) => Value::from(*value),
        AnyValue::Double(value) => Value::from(*value),
        AnyValue::Array(values) => Value::Array(values.iter().map(value_json).collect()),
        AnyValue::Map(values) => Value::Object(attributes(values).into_iter().collect()),
        AnyValue::Bytes(value) => Value::from(value.clone()),
    }
}
