//! Tool timing from normalized Session traces, independent of Harness protocols.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{Call, Time};
use crate::otlp_jsonl::{AnyValue, KeyValue, Logs, Reader, Request, Spans};
use anyhow::{Context, Result};
use serde_json::Value;

fn read<R: Request>(path: &Path) -> Result<Option<Vec<R>>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("can't read {}", path.display())),
    };
    let mut reader = Reader::new(bytes.as_slice()).peekable();
    let mut requests = Vec::new();
    while let Some(result) = reader.next() {
        match result {
            Ok(request) => requests.push(request),
            // A concurrent append may expose only part of the final line.
            // Complete malformed lines must still report their read error.
            Err(error)
                if !bytes.ends_with(b"\n")
                    && reader.peek().is_none()
                    && error
                        .get_ref()
                        .and_then(|error| error.downcast_ref::<serde_json::Error>())
                        .is_some_and(serde_json::Error::is_eof) =>
            {
                break;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("can't read Session trace {}", path.display()));
            }
        }
    }
    Ok(Some(requests))
}

fn attribute<'a>(attrs: &'a [KeyValue], key: &str) -> Option<&'a str> {
    attrs.iter().find_map(|attr| {
        if attr.key == key {
            match &attr.value {
                Some(AnyValue::String(value)) => Some(value.as_str()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn timestamp(nanos: u64) -> Option<Time> {
    if nanos == 0 {
        return None;
    }
    chrono::DateTime::from_timestamp(
        (nanos / 1_000_000_000) as i64,
        (nanos % 1_000_000_000) as u32,
    )
    .map(|time| time.fixed_offset())
}

pub(super) fn calls(session: &Path) -> Result<Option<Vec<Call>>> {
    let session = crate::logs::session::original(session);
    let spans = read::<Spans>(&session.with_extension("spans.jsonl"))?;
    let logs = read::<Logs>(&session.with_extension("logs.jsonl"))?;
    // Historical conversions retain the old Waterfall's scoped Command-log
    // estimates. Their spans are useful to exporters, but aren't measurements.
    if spans.as_ref().is_some_and(|requests| {
        requests
            .iter()
            .flat_map(|r| &r.resource_spans)
            .filter_map(|r| r.resource.as_ref())
            .any(|r| attribute(&r.attributes, "thirdshift.migration") == Some("legacy"))
    }) || logs.as_ref().is_some_and(|requests| {
        requests
            .iter()
            .flat_map(|r| &r.resource_logs)
            .flat_map(|r| &r.scope_logs)
            .flat_map(|s| &s.log_records)
            .any(|r| {
                r.attributes
                    .iter()
                    .any(|a| a.key == "thirdshift.migration.original")
            })
    }) {
        return Ok(None);
    }
    if spans.is_none() && logs.is_none() {
        return Ok(None);
    }
    let mut calls = BTreeMap::new();
    for span in spans
        .into_iter()
        .flatten()
        .flat_map(|request| request.resource_spans)
        .flat_map(|resource| resource.scope_spans)
        .flat_map(|scope| scope.spans)
    {
        let attrs = &span.attributes;
        if attribute(attrs, "gen_ai.operation.name") != Some("execute_tool") {
            continue;
        }
        let Some(id) = attribute(attrs, "gen_ai.tool.call.id") else {
            continue;
        };
        let key = (
            attribute(attrs, "thirdshift.parent_id").map(str::to_owned),
            id.to_owned(),
        );
        let tool = attribute(attrs, "gen_ai.tool.name").unwrap_or(&span.name);
        let label = match attribute(attrs, "gen_ai.tool.call.arguments") {
            Some(input) => format!("{tool} {input}"),
            None => tool.to_owned(),
        };
        calls.insert(
            key,
            Call {
                label,
                start: timestamp(span.start_time_unix_nano),
                end: timestamp(span.end_time_unix_nano),
            },
        );
    }
    for record in logs
        .into_iter()
        .flatten()
        .flat_map(|request| request.resource_logs)
        .flat_map(|resource| resource.scope_logs)
        .flat_map(|scope| scope.log_records)
    {
        let Some(events) = attribute(&record.attributes, "thirdshift.events") else {
            continue;
        };
        let events: Vec<Value> =
            serde_json::from_str(events).context("invalid Session-trace events")?;
        for event in events {
            if event["type"] != "tool_call_started" {
                continue;
            }
            let Some(id) = event["id"].as_str() else {
                continue;
            };
            let key = (
                event["parent_id"].as_str().map(str::to_owned),
                id.to_owned(),
            );
            // A span is authoritative. Record span IDs alone cannot identify
            // every call: one Harness event may start several tools at once.
            calls.entry(key).or_insert_with(|| Call {
                label: format!(
                    "{} {}",
                    event["tool"].as_str().unwrap_or("Tool"),
                    event["input"]
                ),
                start: timestamp(record.time_unix_nano)
                    .or_else(|| timestamp(record.observed_time_unix_nano)),
                end: None,
            });
        }
    }
    let mut calls: Vec<_> = calls.into_values().collect();
    calls.sort_by_key(|call| call.start);
    Ok(Some(calls))
}
