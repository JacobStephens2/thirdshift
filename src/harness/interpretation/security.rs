//! Security session evidence: safeguard refusals are failures even when a
//! Harness reports a successful turn. Raw diagnostics stay in local logs.

use std::fmt;

use serde_json::Value;

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

#[derive(Default)]
pub(super) struct Security {
    pub refusal: Option<SafeguardRefusal>,
    last_model: Option<String>,
    requested_model: Option<String>,
}

impl Security {
    pub fn new(requested_model: Option<&str>) -> Self {
        Self {
            requested_model: requested_model.map(String::from),
            ..Self::default()
        }
    }

    pub fn observe(&mut self, cli: &str, raw: &str) -> Option<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return None;
        };
        match (cli, event["type"].as_str()) {
            ("claude", Some("system"))
                if event["subtype"] == "model_refusal_no_fallback"
                    && event["api_refusal_category"] == "cyber" =>
            {
                self.refusal = Some(SafeguardRefusal::ClaudeCyber);
            }
            ("claude", Some("result"))
                if event["result"].as_str().is_some_and(|text| {
                    let text = text.trim_start();
                    text.starts_with("[cyber]")
                        || (text.starts_with("API Error:") && text.contains("[cyber]"))
                }) =>
            {
                self.refusal = Some(SafeguardRefusal::ClaudeCyber);
            }
            ("codex", Some("turn.failed"))
                if event["error"]["message"]
                    .as_str()
                    .is_some_and(|text| text.to_ascii_lowercase().contains("cybersecurity")) =>
            {
                self.refusal = Some(SafeguardRefusal::CodexCyber);
            }
            _ => {}
        }
        // Init names the requested Model; result.modelUsage includes sub-agents.
        // Only main-loop assistant messages identify the Model that answered.
        if cli == "claude"
            && event["type"] == "assistant"
            && event["parent_tool_use_id"].is_null()
            && let Some(model) = event["message"]["model"].as_str()
            && !model.is_empty()
            && self.last_model.as_deref() != Some(model)
        {
            self.last_model = Some(model.to_string());
            return Some(format!("Model: {model}"));
        }
        if cli == "codex" && event["type"] == "thread.started" && self.last_model.is_none() {
            let model = self.requested_model.as_deref().unwrap_or("Harness default");
            self.last_model = Some(model.to_string());
            let basis = if self.requested_model.is_some() {
                "requested"
            } else {
                "no Model requested"
            };
            return Some(format!("Model: {model} ({basis})"));
        }
        None
    }
}
