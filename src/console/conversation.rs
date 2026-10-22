//! Render normalized Session-trace events, with legacy Claude/Codex support,
//! as one escaped conversation with each `https://` URL a link.
use super::filters::{escaped, linked};
use crate::logs::catalog::Kind;
use crate::otlp_jsonl::{AnyValue, LogRecord};
use serde_json::Value;

enum Part {
    Message(String, String),
    Thinking(String),
    Tool(String, String),
    Plain(String),
}

pub(super) fn render(lines: &str, kind: Kind) -> String {
    if matches!(kind, Kind::Command) {
        return format!("<pre class=\"command-output\">{}</pre>", linked(lines));
    }
    let mut html = String::new();
    let mut tools_open = false;
    for part in crate::logs::session::records(lines).flat_map(record_parts) {
        if matches!(part, Part::Tool(..)) {
            if !tools_open {
                html.push_str(
                    "<details class=\"tool-group\"><summary>Tools</summary><div class=\"tools\">",
                );
                tools_open = true;
            }
        } else if tools_open {
            html.push_str("</div></details>");
            tools_open = false;
        }
        match part {
            Part::Message(role, text) => html.push_str(&format!(
                "<article class=\"message\"><h3>{}</h3><pre>{}</pre></article>",
                escaped(&role),
                linked(&text)
            )),
            Part::Thinking(text) => html.push_str(&format!(
                "<details class=\"thinking\"><summary>Thinking</summary><pre>{}</pre></details>",
                linked(&text)
            )),
            Part::Tool(name, body) => html.push_str(&format!(
                "<article class=\"tool\" data-tool=\"{}\"><h3>{}</h3>{body}</article>",
                escaped(&name),
                escaped(&name)
            )),
            Part::Plain(text) => html.push_str(&format!(
                "<pre class=\"unknown-event\">{}</pre>",
                linked(&text)
            )),
        }
    }
    if tools_open {
        html.push_str("</div></details>");
    }
    html
}

fn record_parts(record: LogRecord) -> Vec<Part> {
    let Some(AnyValue::String(raw)) = record.body else {
        return Vec::new();
    };
    let Some(attribute) = record
        .attributes
        .iter()
        .find(|attr| attr.key == "thirdshift.events")
    else {
        return parts(&raw);
    };
    let Some(AnyValue::String(events)) = &attribute.value else {
        return vec![Part::Plain(raw)];
    };
    let Ok(Value::Array(events)) = serde_json::from_str(events) else {
        return vec![Part::Plain(raw)];
    };
    let mut output = Vec::new();
    for event in events {
        match event["type"].as_str() {
            Some("message") if event["role"].is_string() && event["text"].is_string() => {
                output.push(Part::Message(
                    text(&event, "role").into(),
                    text(&event, "text").into(),
                ));
            }
            Some("tool_call_started" | "tool_call_ended") if event["tool"].is_string() => {
                let mut part = tool(text(&event, "tool"), &event["input"]);
                if event["type"] == "tool_call_ended"
                    && let Part::Tool(_, body) = &mut part
                {
                    body.push_str(&pre("result", &pretty(&event["result"])));
                }
                output.push(part);
            }
            Some(
                "model_call_started" | "model_call_ended" | "turn_started" | "turn_ended" | "usage"
                | "error" | "work_ended" | "subagent_started" | "subagent_ended",
            ) => {}
            _ => output.push(Part::Plain(raw.clone())),
        }
    }
    if output.is_empty() {
        // Thinking and other raw-only content still use the legacy renderer.
        parts(&raw)
    } else {
        // Messages normalization has no thinking/unknown-block facts. Retain
        // those blocks alongside normalized content without repeating text or
        // tools from the raw message envelope.
        if serde_json::from_str::<Value>(&raw)
            .is_ok_and(|event| matches!(event["type"].as_str(), Some("assistant" | "user")))
        {
            output.extend(
                parts(&raw)
                    .into_iter()
                    .filter(|part| matches!(part, Part::Thinking(_) | Part::Plain(_))),
            );
        }
        output
    }
}

fn parts(line: &str) -> Vec<Part> {
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return vec![Part::Plain(line.into())];
    };
    match event["type"].as_str() {
        Some("assistant" | "user") => {
            let role = text(&event, "type");
            match &event["message"]["content"] {
                Value::String(content) => vec![Part::Message(role.into(), content.clone())],
                Value::Array(blocks) if !blocks.is_empty() => blocks
                    .iter()
                    .map(|block| match block["type"].as_str() {
                        Some("text") => Part::Message(role.into(), text(block, "text").into()),
                        Some("thinking") => Part::Thinking(text(block, "thinking").into()),
                        Some("tool_use") => tool(text(block, "name"), &block["input"]),
                        Some("tool_result") => Part::Tool(
                            "Tool result".into(),
                            pre("result", &pretty(&block["content"])),
                        ),
                        _ => Part::Plain(pretty(block)),
                    })
                    .collect(),
                _ => vec![Part::Plain(line.into())],
            }
        }
        Some("item.started" | "item.updated" | "item.completed") => {
            let item = &event["item"];
            vec![match item["type"].as_str() {
                Some("agent_message") => {
                    Part::Message("assistant".into(), text(item, "text").into())
                }
                Some("reasoning") => Part::Thinking(text(item, "text").into()),
                Some("command_execution") => tool("Bash", item),
                Some("file_change") => tool("Edit", item),
                Some("mcp_tool_call") => tool(text(item, "tool"), item),
                Some("web_search") => tool("Grep", item),
                _ => Part::Plain(line.into()),
            }]
        }
        _ => vec![Part::Plain(line.into())],
    }
}

fn tool(name: &str, input: &Value) -> Part {
    let arguments = input.get("arguments").unwrap_or(input);
    let body = match name.to_ascii_lowercase().as_str() {
        "bash" | "exec_command" => format!(
            "{}{}",
            pre("bash", text(input, "command")),
            pre("result", &pretty(input))
        ),
        "edit" | "apply_patch" => {
            if input["changes"].is_array() {
                // Codex exec records paths and kinds, not before/after text.
                // Keep the actual event visible rather than inventing a patch.
                return Part::Tool(
                    name.into(),
                    format!(
                        "<p>Patch unavailable in this log.</p>{}",
                        pre("fallback", &pretty(input))
                    ),
                );
            }
            let diff = if arguments["old_string"].is_string() && arguments["new_string"].is_string()
            {
                let path = text(arguments, "file_path");
                let old = text(arguments, "old_string");
                let new = text(arguments, "new_string");
                format!(
                    "--- a/{path}\n+++ b/{path}\n@@ -1,{} +1,{} @@\n{}{}",
                    old.lines().count(),
                    new.lines().count(),
                    old.lines()
                        .map(|line| format!("-{line}\n"))
                        .collect::<String>(),
                    new.lines()
                        .map(|line| format!("+{line}\n"))
                        .collect::<String>()
                )
            } else {
                return Part::Tool(name.into(), pre("fallback", &pretty(input)));
            };
            pre("diff", &diff)
        }
        "read" | "read_file" => format!(
            "{}{}",
            pre("read", text(arguments, "file_path")),
            pre("result", &pretty(input))
        ),
        "grep" | "search" => format!(
            "{}{}",
            pre("grep", text(arguments, "pattern")),
            pre("result", &pretty(input))
        ),
        _ => pre("fallback", &pretty(input)),
    };
    Part::Tool(name.into(), body)
}

fn pre(class: &str, text: &str) -> String {
    format!("<pre class=\"{class}\">{}</pre>", linked(text))
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn pretty(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
}
