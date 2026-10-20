//! Console settings stay in the User config; serving never writes them.
use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    pub port: u16,
    pub allowed_emails: Vec<String>,
    pub other_consoles: Vec<String>,
    pub theme: String,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            port: 8711,
            allowed_emails: Vec::new(),
            other_consoles: Vec::new(),
            theme: "focus-auto".into(),
        }
    }
}

impl ConsoleSettings {
    pub(super) fn validate(&self) -> Result<()> {
        for email in &self.allowed_emails {
            ensure!(
                email.contains('@') && !email.chars().any(char::is_whitespace),
                "console.allowed_emails must contain email addresses"
            );
        }
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
