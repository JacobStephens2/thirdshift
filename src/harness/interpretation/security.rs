//! Security session evidence: safeguard refusals are failures even when a
//! Harness reports a successful turn. Raw diagnostics stay in local logs.

use std::fmt;

use super::{Evidence, ModelScope};

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
    pub refusal: Option<SafeguardRefusal>,
    last_model: Option<String>,
    requested_model: Option<String>,
}

impl Security {
    pub fn new(requested_model: Option<&str>) -> Self {
        Self {
            refusal: None,
            last_model: None,
            requested_model: requested_model.map(String::from),
        }
    }

    pub fn apply(&mut self, evidence: &Evidence) -> Option<String> {
        match evidence {
            Evidence::SafeguardRefusal(refusal) => {
                self.refusal = Some(*refusal);
                None
            }
            Evidence::RequestedModelFallback if self.last_model.is_none() => {
                let model = self.requested_model.as_deref().unwrap_or("Harness default");
                let basis = if self.requested_model.is_some() {
                    "requested"
                } else {
                    "no Model requested"
                };
                self.last_model = Some(model.to_string());
                Some(format!("Model: {model} ({basis})"))
            }
            Evidence::ObservedModel {
                name,
                scope: ModelScope::MainLoop,
            } if self.last_model.as_deref() != Some(name) => {
                self.last_model = Some(name.clone());
                Some(format!("Model: {name}"))
            }
            _ => None,
        }
    }
}
