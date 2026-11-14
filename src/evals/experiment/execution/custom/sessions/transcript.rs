//! SDK transcripts preserve text and normalize tool calls before Trial checks.

use anyhow::Result;
use serde_json::Value;

use crate::harness::interpretation::event::{Event, Usage};
use crate::session::trace::Trace;

pub(super) fn record(text: &str, trace: &mut Trace, started: u64) -> Result<bool> {
    let mut web_search = false;
    let mut next = 0;
    // A transcript can be one JSON document, JSONL records, or plain text.
    let records: Vec<_> = if serde_json::from_str::<Value>(text).is_ok() {
        vec![text]
    } else {
        text.lines().collect()
    };
    for raw in records {
        let mut events = Vec::new();
        if let Ok(value) = serde_json::from_str(raw) {
            normalize(&value, &mut events, &mut next);
        }
        if events.is_empty() {
            events.push(message("assistant", raw));
        }
        web_search |= super::web_search(&events);
        trace.record_at(raw.as_bytes(), &events, started)?;
    }
    Ok(web_search)
}

fn normalize(value: &Value, events: &mut Vec<Event>, next: &mut usize) {
    if let Ok(mut event) = serde_json::from_value::<Event>(value.clone()) {
        // Session models in sessions.json alone supply SDK usage. Transcript
        // usage and USD cannot override that cache-inclusive measurement.
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only usage-bearing events need their measurements removed"
        )]
        match &mut event {
            Event::Usage { .. } => return,
            Event::ModelCallEnded {
                usage, cost_usd, ..
            } => {
                *usage = Usage::default();
                *cost_usd = None;
            }
            Event::TurnEnded { usage, .. } => *usage = Usage::default(),
            _ => {}
        }
        events.push(event);
        return;
    }
    if let Some(values) = value.as_array() {
        for value in values {
            normalize(value, events, next);
        }
        return;
    }
    let kind = value["type"].as_str();
    let tool = match kind {
        Some("web_search_call" | "web_search_preview") => Some("web_search"),
        Some("tool_use" | "function_call" | "tool_call") => {
            value["name"].as_str().or_else(|| value["tool"].as_str())
        }
        _ => None,
    };
    if let Some(tool) = tool {
        let input = value
            .get("input")
            .or_else(|| value.get("arguments"))
            .or_else(|| value.get("action"))
            .cloned()
            .unwrap_or(Value::Null);
        tool_call(value, tool, input, events, next);
    }
    if let Some(calls) = value["tool_calls"].as_array() {
        for call in calls {
            if let Some(tool) = call["function"]["name"].as_str() {
                tool_call(
                    call,
                    tool,
                    call["function"]["arguments"].clone(),
                    events,
                    next,
                );
            }
        }
    }
    let role = value["role"].as_str().unwrap_or("assistant");
    if let Some(text) = value["content"].as_str().or_else(|| value["text"].as_str()) {
        events.push(message(role, text));
    }
    for key in ["content", "message", "messages", "output"] {
        if let Some(nested) = value
            .get(key)
            .filter(|value| value.is_array() || value.is_object())
        {
            normalize(nested, events, next);
        }
    }
}

fn tool_call(value: &Value, tool: &str, input: Value, events: &mut Vec<Event>, next: &mut usize) {
    *next += 1;
    let input = input
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(input);
    events.push(Event::ToolCallStarted {
        id: value["id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("sdk-tool-{next}")),
        parent_id: None,
        tool: tool.into(),
        input,
    });
}

fn message(role: &str, text: &str) -> Event {
    Event::Message {
        id: None,
        role: role.into(),
        text: text.into(),
        parent_id: None,
    }
}
