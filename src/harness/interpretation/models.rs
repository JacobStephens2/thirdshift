//! Models named by session evidence, including delegated Claude answers.
//! Requested defaults are reported by the session owner, never as observations.

use serde_json::Value;

use crate::harness::Harness;

pub(super) struct Models {
    harness: Option<Harness>,
    names: Vec<String>,
}

impl Models {
    pub fn new(cli: &str) -> Self {
        Self {
            harness: Harness::named(cli),
            names: Vec::new(),
        }
    }

    pub fn observe(&mut self, raw: &str) {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        let model = match self.harness {
            Some(Harness::Claude | Harness::Grok)
                if event["type"] == "assistant" && event["is_api_error_message"] != true =>
            {
                event["message"]["model"].as_str()
            }
            Some(Harness::Agy) if event["event"] == "init" => event["init"]["model"].as_str(),
            _ => None,
        };
        if let Some(model) = model {
            self.include(model);
        }
        if self.harness == Some(Harness::Claude)
            && event["type"] == "result"
            && let Some(usage) = event["modelUsage"].as_object()
        {
            for model in usage.keys() {
                self.include(model);
            }
        }
    }

    fn include(&mut self, model: &str) {
        if !model.is_empty()
            && model != "<synthetic>"
            && !self.names.iter().any(|name| name == model)
        {
            self.names.push(model.to_string());
        }
    }

    pub fn into_names(self) -> Vec<String> {
        self.names
    }
}
