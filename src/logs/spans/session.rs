//! Merge ended OTLP spans with normalized start records for live Sessions.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::{Span, duration, hex};
use crate::logs::trace::{Context, id};
use crate::otlp_jsonl::{AnyValue, KeyValue, Logs, Spans};

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
                span.kind = kind(&attrs).into();
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
        .find(|span| span.kind == "sess")
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
        .find(|span| span.kind == "sess")
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
    let mut calls = BTreeMap::new();
    let mut locations = HashMap::new();
    let identities: HashMap<_, _> = completed
        .iter()
        .filter_map(|span| {
            let call = span
                .attributes
                .get("gen_ai.tool.call.id")
                .or_else(|| span.attributes.get("thirdshift.id"))?
                .as_str()?;
            let parent = span
                .attributes
                .get("thirdshift.parent_id")
                .and_then(Value::as_str);
            Some((
                serde_json::to_string(&(&span.kind, parent, call)).unwrap(),
                span.id.clone(),
            ))
        })
        .collect();
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
                let start = millis(record.time_unix_nano)
                    .or_else(|| millis(record.observed_time_unix_nano));
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
                    let event_type = event["type"].as_str().unwrap_or_default();
                    let call_kind = match event_type {
                        "tool_call_started" | "tool_call_ended" => "tool",
                        "model_call_started" | "model_call_ended" => "model",
                        "turn_started" | "turn_ended" => "turn",
                        "subagent_started" | "subagent_ended" => "agent",
                        _ => continue,
                    };
                    let Some(call) = event["id"].as_str() else {
                        continue;
                    };
                    let parent = event["parent_id"].as_str();
                    let key = serde_json::to_string(&(call_kind, parent, call))?;
                    let call_id = if call_kind == "agent" && event_type.ends_with("_ended") {
                        locations
                            .get(call)
                            .cloned()
                            .unwrap_or_else(|| hex(id(&[&session_bytes.0, key.as_bytes()])))
                    } else {
                        identities
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| hex(id(&[&session_bytes.0, key.as_bytes()])))
                    };
                    let parent_id = parent
                        .and_then(|parent| locations.get(parent).cloned())
                        .unwrap_or_else(|| session_id.clone());
                    locations.insert(call.to_owned(), call_id.clone());
                    let span = calls.entry(call_id.clone()).or_insert_with(|| {
                        let mut span = template.clone();
                        span.id = call_id;
                        span.parent_id = Some(parent_id);
                        span.kind = call_kind.into();
                        span.name = format!("{call_kind} {call}");
                        span.start = start;
                        span.end = None;
                        span.duration_ms = None;
                        span.error = None;
                        span.status = if template.status == "running" {
                            "running"
                        } else {
                            "unknown"
                        }
                        .into();
                        span.attributes.clear();
                        span
                    });
                    let mut own_attrs = resource_attrs.clone();
                    own_attrs.insert("thirdshift.type".into(), Value::String(event_type.into()));
                    for (key, value) in event.as_object().into_iter().flatten() {
                        if !value.is_null() {
                            own_attrs.insert(format!("thirdshift.{key}"), value.clone());
                        }
                    }
                    if call_kind == "tool" {
                        own_attrs.insert("gen_ai.tool.name".into(), event["tool"].clone());
                        if !event["input"].is_null() {
                            own_attrs.insert(
                                "gen_ai.tool.call.arguments".into(),
                                event["input"].clone(),
                            );
                        }
                        if !event["result"].is_null() {
                            own_attrs
                                .insert("gen_ai.tool.call.result".into(), event["result"].clone());
                        }
                    }
                    if event["model"].is_string() {
                        own_attrs.insert("gen_ai.response.model".into(), event["model"].clone());
                    }
                    if attrs.get("thirdshift.time.estimated") == Some(&Value::Bool(true)) {
                        own_attrs.insert("thirdshift.time.estimated".into(), Value::Bool(true));
                    }
                    span.attributes.extend(own_attrs.clone());
                    apply_attributes(span, span.attributes.clone());
                    if event_type.ends_with("_ended") {
                        span.end = span.end.or(start);
                        span.duration_ms = duration(span.start, span.end);
                        if event["outcome"] != "succeeded" {
                            span.status = "error".into();
                            span.error = Some(
                                event["outcome"]
                                    .as_str()
                                    .unwrap_or("call did not end")
                                    .into(),
                            );
                        } else {
                            span.status = "ok".into();
                        }
                    }
                }
            }
        }
    }
    if !completed.iter().any(|span| span.kind == "sess") {
        session.start = first_record.or(session.start);
        session.duration_ms = duration(session.start, session.end);
        apply_attributes(&mut session, resource_attrs);
    }
    for mut span in completed.into_iter().filter(|span| span.kind != "sess") {
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

fn kind(attrs: &BTreeMap<String, Value>) -> &'static str {
    match attrs.get("gen_ai.operation.name").and_then(Value::as_str) {
        Some("invoke_agent") => "sess",
        Some("execute_tool") => "tool",
        Some("chat") => "model",
        _ => match attrs
            .get("thirdshift.type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            value if value.starts_with("subagent_") => "agent",
            value if value.starts_with("turn_") => "turn",
            _ => "model",
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
    if span.kind == "tool"
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
