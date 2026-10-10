//! Models named by session evidence, including delegated Claude answers.
//! Requested defaults are reported by the session owner, never as observations.

#[derive(Default)]
pub(super) struct Models {
    names: Vec<String>,
}

impl Models {
    /// Accept usable observed names, preserving first-observed order. Repeated
    /// names remain eligible for Security progress without duplicating attribution.
    pub fn include(&mut self, model: &str) -> bool {
        if model.is_empty() || model == "<synthetic>" {
            return false;
        }
        if !self.names.iter().any(|name| name == model) {
            self.names.push(model.to_string());
        }
        true
    }

    pub fn into_names(self) -> Vec<String> {
        self.names
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}
