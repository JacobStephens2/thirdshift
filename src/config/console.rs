//! Console settings stay in the User config; serving never writes them.
use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    pub port: u16,
    pub other_consoles: Vec<String>,
    pub theme: String,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            port: 8711,
            other_consoles: Vec::new(),
            theme: "focus-auto".into(),
        }
    }
}

impl ConsoleSettings {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            matches!(
                self.theme.as_str(),
                "focus-auto" | "focus-light" | "focus-dark"
            ),
            "console.theme must be focus-auto, focus-light or focus-dark"
        );
        for address in &self.other_consoles {
            let url = reqwest::Url::parse(address)?;
            ensure!(
                matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                "console.other_consoles must contain http or https addresses"
            );
        }
        Ok(())
    }
}
