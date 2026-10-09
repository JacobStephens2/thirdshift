//! Claude and Grok share stream framing, with private dialect rules.
//!
//! A result ends a turn, not necessarily the session: background sub-agents
//! can resume it. Only the first init sets shared session fields, and only
//! completion supplies the last result and any work killed since that result.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use super::{Decoder, Ended, Facts, Report, TurnOutcome};
use crate::progress::{bash, shorten};

pub struct Stream {
    init: Option<Init>,
    /// Cumulative totals survive later results without a complete pair.
    turns_and_cost: Option<(u64, f64)>,
    final_message: Option<String>,
    dialect: Dialect,
}

struct Init {
    cwd: Option<String>,
    session_id: Option<String>,
    subscription: bool,
}

enum Dialect {
    Claude(Claude),
    Grok(Grok),
}

#[derive(Default)]
struct Claude {
    failed: bool,
    diagnostic: Option<String>,
    /// The latest root API error, only until this turn's result.
    api_error: Option<ClaudeApiError>,
    tasks: HashMap<String, String>,
    /// Task id and description, in order of the first kill since the result.
    killed: Vec<(String, String)>,
}

#[derive(Default)]
struct ClaudeApiError {
    message: Option<String>,
    /// Only diagnostic fields, never unrelated raw event content.
    facts: BTreeMap<&'static str, String>,
}

#[derive(Default)]
struct Grok {
    failed: bool,
    diagnostic: Option<String>,
    fallback_id: Option<String>,
    usage: Option<String>,
    cost_known: bool,
}

impl Stream {
    pub fn claude() -> Self {
        Self::new(Dialect::Claude(Claude::default()))
    }

    pub fn grok() -> Self {
        Self::new(Dialect::Grok(Grok::default()))
    }

    fn new(dialect: Dialect) -> Self {
        Self {
            init: None,
            turns_and_cost: None,
            final_message: None,
            dialect,
        }
    }

    fn summary(&self) -> Option<String> {
        let totals = || {
            self.turns_and_cost.map(|(turns, cost)| {
                let basis = if self.init.as_ref().is_some_and(|init| init.subscription) {
                    " at API prices"
                } else {
                    ""
                };
                format!("{turns} turns, ${cost:.2}{basis}")
            })
        };
        match &self.dialect {
            Dialect::Claude(_) => totals(),
            Dialect::Grok(grok) if grok.cost_known => totals().or_else(|| grok.usage.clone()),
            Dialect::Grok(grok) => grok.usage.clone(),
        }
    }

    fn tool_use(&self, name: &str, input: &Value) -> String {
        let grok = matches!(self.dialect, Dialect::Grok(_));
        let name = if grok && matches!(name, "bash" | "run_terminal_command") {
            "Bash"
        } else {
            name
        };
        match name {
            "Skill" => format!("skill {}", input["skill"].as_str().unwrap_or("?")),
            "Bash" => bash(input["command"].as_str().unwrap_or("")),
            _ => {
                let path = if grok { input["path"].as_str() } else { None };
                let detail = path.or_else(|| {
                    ["file_path", "pattern", "description", "url", "query"]
                        .iter()
                        .find_map(|key| input[key].as_str())
                });
                match detail {
                    Some(detail) => format!("{name} {}", shorten(self.relative(detail))),
                    None => name.to_string(),
                }
            }
        }
    }

    fn relative<'a>(&self, path: &'a str) -> &'a str {
        self.init
            .as_ref()
            .and_then(|init| init.cwd.as_deref())
            .and_then(|cwd| path.strip_prefix(cwd)?.strip_prefix('/'))
            .unwrap_or(path)
    }
}

impl Decoder for Stream {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        if let Dialect::Grok(grok) = &mut self.dialect
            && let Some(id) = event["session_id"].as_str()
        {
            grok.fallback_id.get_or_insert_with(|| id.to_string());
        }
        let mut lines = match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" && self.init.is_none() => {
                self.init = Some(Init {
                    cwd: event["cwd"].as_str().map(String::from),
                    session_id: event["session_id"].as_str().map(String::from),
                    subscription: event["apiKeySource"] == "none",
                });
                vec!["session started".to_string()]
            }
            Some("assistant") => {
                if let Dialect::Claude(claude) = &mut self.dialect {
                    claude.assistant(&event);
                }
                event["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "tool_use")
                    .filter_map(|block| {
                        Some(self.tool_use(block["name"].as_str()?, &block["input"]))
                    })
                    .collect()
            }
            Some("system") => {
                if let Dialect::Claude(claude) = &mut self.dialect {
                    claude.track_task(&event);
                }
                Vec::new()
            }
            Some("result") => {
                if matches!(self.dialect, Dialect::Claude(_))
                    && !event["parent_tool_use_id"].is_null()
                {
                    return Vec::new();
                }
                self.final_message = event["result"].as_str().map(String::from);
                if let (Some(turns), Some(cost)) = (
                    event["num_turns"].as_u64(),
                    event["total_cost_usd"].as_f64(),
                ) {
                    self.turns_and_cost = Some((turns, cost));
                }
                let failed = event["subtype"] != "success" || event["is_error"] == true;
                match &mut self.dialect {
                    Dialect::Claude(claude) => {
                        claude.result(&event, failed);
                    }
                    Dialect::Grok(grok) => grok.result(&event, failed),
                }
                Vec::new()
            }
            Some("error") => {
                if let Dialect::Grok(grok) = &mut self.dialect {
                    grok.failed = true;
                    grok.diagnostic = event["message"].as_str().map(String::from);
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        if let Dialect::Grok(grok) = &self.dialect
            && grok.failed
            && matches!(event["type"].as_str(), Some("error" | "result"))
        {
            lines.push(format!(
                "error: {}",
                shorten(grok.diagnostic.as_deref().unwrap_or("Grok's turn failed"))
            ));
        }
        lines
    }

    fn complete(self: Box<Self>) -> Facts {
        let summary = self.summary();
        let mut ended = Ended {
            session_id: self.init.and_then(|init| init.session_id),
            killed: Vec::new(),
            final_message: self.final_message,
        };
        let (failed, diagnostic) = match self.dialect {
            Dialect::Claude(claude) => {
                ended.killed = claude.killed.into_iter().map(|(_, text)| text).collect();
                let diagnostic = claude
                    .api_error
                    .and_then(|error| error.diagnostic(None))
                    .or(claude.diagnostic);
                (claude.failed, diagnostic)
            }
            Dialect::Grok(grok) => {
                ended.session_id = ended.session_id.or(grok.fallback_id);
                (grok.failed, grok.diagnostic)
            }
        };
        Facts {
            report: Report {
                warnings: Vec::new(),
                summary,
                ..Report::default()
            },
            ended,
            outcome: TurnOutcome::from_failed(failed),
            diagnostic,
        }
    }
}

impl Claude {
    fn assistant(&mut self, event: &Value) {
        if !event["parent_tool_use_id"].is_null() {
            return;
        }
        self.api_error = (event["is_api_error_message"] == true
            || nonempty_text(&event["error"]).is_some()
            || nonempty_text(&event["api_error"]).is_some())
        .then(|| ClaudeApiError::read(event));
    }

    fn result(&mut self, event: &Value, failed: bool) {
        self.failed = failed;
        self.killed.clear();
        let mut api_error = self.api_error.take().unwrap_or_default();
        self.diagnostic = if failed {
            // Terminal facts replace only the fields they actually supply.
            api_error.facts.extend(ClaudeApiError::facts(event));
            let errors: Vec<_> = event["errors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(nonempty_text)
                .collect();
            let message = if errors.is_empty() {
                nonempty_text(&event["result"]).map(String::from)
            } else {
                Some(errors.join("\n"))
            };
            api_error.diagnostic(message)
        } else {
            None
        };
    }

    fn track_task(&mut self, event: &Value) {
        let Some(id) = event["task_id"].as_str() else {
            return;
        };
        let killed = match event["subtype"].as_str() {
            Some("task_started") => {
                if let Some(description) = event["description"].as_str() {
                    self.tasks.insert(id.to_string(), description.to_string());
                }
                false
            }
            Some("task_updated") => event["patch"]["status"] == "killed",
            Some("task_notification") => event["status"] == "stopped",
            _ => false,
        };
        if killed && !self.killed.iter().any(|(killed_id, _)| killed_id == id) {
            let description = self
                .tasks
                .get(id)
                .map(String::as_str)
                .or(event["summary"].as_str())
                .unwrap_or(id)
                .to_string();
            self.killed.push((id.to_string(), description));
        }
    }
}

impl ClaudeApiError {
    fn read(event: &Value) -> Self {
        let text: Vec<_> = event["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| nonempty_text(&block["text"]))
            .collect();
        Self {
            message: (!text.is_empty()).then(|| text.join("\n")),
            facts: Self::facts(event),
        }
    }

    fn facts(event: &Value) -> BTreeMap<&'static str, String> {
        let mut facts = BTreeMap::new();
        if let Some(kind) = nonempty_text(&event["api_error"]) {
            facts.insert("api_error", kind.to_string());
        }
        if let Some(status) = event["api_error_status"].as_u64() {
            facts.insert("api_error_status", status.to_string());
        }
        let limit = &event["api_error_params"]["rate_limit_info"];
        for key in [
            "status",
            "rateLimitType",
            "overageStatus",
            "overageDisabledReason",
        ] {
            if let Some(text) = nonempty_text(&limit[key]) {
                facts.insert(key, text.to_string());
            }
        }
        for key in ["resetsAt", "overageResetsAt"] {
            if let Some(timestamp) = limit[key].as_u64() {
                facts.insert(key, timestamp.to_string());
            }
        }
        facts
    }

    fn diagnostic(self, message: Option<String>) -> Option<String> {
        let message = message.or(self.message);
        if self.facts.is_empty() {
            return message;
        }
        let facts = self
            .facts
            .into_iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("; ");
        Some(format!(
            "{}\n{facts}",
            message.as_deref().unwrap_or("Claude's turn failed")
        ))
    }
}

fn nonempty_text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

impl Grok {
    fn result(&mut self, event: &Value, failed: bool) {
        self.failed = failed;
        // Zero is a placeholder for unknown cost in Messages streams.
        self.cost_known = event["total_cost_usd"]
            .as_f64()
            .is_some_and(|cost| cost > 0.0);
        let errors: Vec<_> = event["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !errors.is_empty() {
            self.diagnostic = Some(errors.join("\n"));
        } else if failed
            && let Some(text) = event["result"].as_str().filter(|text| !text.is_empty())
        {
            self.diagnostic = Some(text.to_string());
        }
        self.usage = match (
            event["usage"]["input_tokens"].as_u64(),
            event["usage"]["output_tokens"].as_u64(),
        ) {
            (Some(input), Some(output)) => {
                let cached = event["usage"]["cache_read_input_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                let created = event["usage"]["cache_creation_input_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                Some(format!(
                    "{input} input tokens, {cached} cache read tokens, {created} cache creation tokens, {output} output tokens"
                ))
            }
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests;
