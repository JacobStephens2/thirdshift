//! Security session evidence: safeguard refusals are failures even when a
//! Harness reports a successful turn. Raw diagnostics stay in local logs.

use std::fmt;

use serde_json::Value;

use crate::harness::Harness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeguardRefusal {
    ClaudeCyber,
    CodexCyber,
}

impl SafeguardRefusal {
    pub fn description(self) -> &'static str {
        match self {
            Self::ClaudeCyber => "Claude Code's [cyber] safeguard refusal",
            Self::CodexCyber => "Codex's cybersecurity safeguard refusal",
        }
    }
}

impl fmt::Display for SafeguardRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl std::error::Error for SafeguardRefusal {}

pub(super) struct Security {
    harness: Option<Harness>,
    pub refusal: Option<SafeguardRefusal>,
    last_model: Option<String>,
    requested_model: Option<String>,
}

impl Security {
    pub fn new(cli: &str, requested_model: Option<&str>) -> Self {
        Self {
            harness: Harness::named(cli),
            refusal: None,
            last_model: None,
            requested_model: requested_model.map(String::from),
        }
    }

    pub fn observe(&mut self, raw: &str) -> Option<String> {
        let event = serde_json::from_str::<Value>(raw).ok()?;
        let model = match self.harness {
            Some(Harness::Claude) => {
                let refused = match event["type"].as_str() {
                    Some("system") => {
                        event["subtype"] == "model_refusal_no_fallback"
                            && event["api_refusal_category"] == "cyber"
                    }
                    Some("result") => event["result"].as_str().is_some_and(|text| {
                        let text = text.trim_start();
                        text.starts_with("[cyber]")
                            || (text.starts_with("API Error:") && text.contains("[cyber]"))
                    }),
                    _ => false,
                };
                if refused {
                    self.refusal = Some(SafeguardRefusal::ClaudeCyber);
                }
                Self::answer_model(&event)
            }
            Some(Harness::Grok) => Self::answer_model(&event),
            Some(Harness::Codex) => {
                if event["type"] == "turn.failed"
                    && event["error"]["message"]
                        .as_str()
                        .is_some_and(|text| text.to_ascii_lowercase().contains("cybersecurity"))
                {
                    self.refusal = Some(SafeguardRefusal::CodexCyber);
                }
                if event["type"] == "thread.started" && self.last_model.is_none() {
                    let model = self.requested_model.as_deref().unwrap_or("Harness default");
                    let basis = if self.requested_model.is_some() {
                        "requested"
                    } else {
                        "no Model requested"
                    };
                    let line = format!("Model: {model} ({basis})");
                    self.last_model = Some(model.to_string());
                    return Some(line);
                }
                None
            }
            Some(Harness::Agy) if event["event"] == "init" => event["init"]["model"].as_str(),
            _ => None,
        }?;
        if model.is_empty() || self.last_model.as_deref() == Some(model) {
            return None;
        }
        self.last_model = Some(model.to_string());
        Some(format!("Model: {model}"))
    }

    // Init names the requested Model; result.modelUsage includes sub-agents.
    // Only main-loop assistant messages identify the Model that answered.
    fn answer_model(event: &Value) -> Option<&str> {
        if event["type"] != "assistant"
            || !event["parent_tool_use_id"].is_null()
            || event["is_api_error_message"] == true
        {
            return None;
        }
        event["message"]["model"]
            .as_str()
            .filter(|model| *model != "<synthetic>")
    }
}
