use crate::harness::interpretation::event::{Event, Usage};
use crate::otlp_jsonl::{AnyValue, KeyValue};
use serde_json::Value;

pub(super) fn attribute(key: &str, value: impl ToString) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue::String(value.to_string())),
    }
}

fn usage_attributes(
    attrs: &mut Vec<KeyValue>,
    usage: &Usage,
    cost: Option<f64>,
    input_excludes_cache: bool,
) {
    // Claude/Grok's Messages counters exclude cache reads and writes. The
    // other normalized protocols already report inclusive input totals.
    let input = usage.input.map(|input| {
        if input_excludes_cache {
            input
                .saturating_add(usage.cache_read.unwrap_or(0))
                .saturating_add(usage.cache_write.unwrap_or(0))
        } else {
            input
        }
    });
    for (key, value) in [
        ("gen_ai.usage.input_tokens", input),
        ("gen_ai.usage.output_tokens", usage.output),
        ("thirdshift.usage.cache_read_tokens", usage.cache_read),
        ("thirdshift.usage.cache_write_tokens", usage.cache_write),
    ] {
        if let Some(value) = value {
            attrs.push(KeyValue {
                key: key.into(),
                value: Some(
                    i64::try_from(value)
                        .map(AnyValue::Int)
                        .unwrap_or_else(|_| AnyValue::String(value.to_string())),
                ),
            });
        }
    }
    if let Some(cost) = cost {
        attrs.push(KeyValue {
            key: "thirdshift.cost.usd".into(),
            value: Some(AnyValue::Double(cost)),
        });
    }
}

pub(crate) fn event_attributes(event: &Event, input_excludes_cache: bool) -> Vec<KeyValue> {
    use Event::*;
    let mut attrs = Vec::new();
    let json = serde_json::to_value(event).unwrap();
    attrs.push(attribute(
        "thirdshift.event",
        serde_json::to_string(&json).unwrap(),
    ));
    for (key, value) in json.as_object().unwrap() {
        if value.is_null() {
            continue;
        }
        attrs.push(attribute(
            &format!("thirdshift.{key}"),
            match value {
                Value::String(text) => text.clone(),
                _ => value.to_string(),
            },
        ));
    }
    match event {
        ModelCallStarted { model, .. } | ModelCallEnded { model, .. } => {
            attrs.push(attribute("gen_ai.operation.name", "chat"));
            if let Some(model) = model {
                attrs.push(attribute("gen_ai.response.model", model));
            }
        }
        ToolCallStarted {
            tool, id, input, ..
        }
        | ToolCallEnded {
            tool, id, input, ..
        } => {
            attrs.push(attribute("gen_ai.operation.name", "execute_tool"));
            attrs.push(attribute("gen_ai.tool.name", tool));
            attrs.push(attribute("gen_ai.tool.call.id", id));
            attrs.push(attribute("gen_ai.tool.call.arguments", input));
            if let ToolCallEnded { result, .. } = event {
                attrs.push(attribute("gen_ai.tool.call.result", result));
            }
        }
        _ => {}
    }
    match event {
        ModelCallEnded {
            usage, cost_usd, ..
        }
        | Usage {
            usage, cost_usd, ..
        } => usage_attributes(&mut attrs, usage, *cost_usd, input_excludes_cache),
        TurnEnded { usage, .. } => usage_attributes(&mut attrs, usage, None, input_excludes_cache),
        _ => {}
    }
    attrs
}
