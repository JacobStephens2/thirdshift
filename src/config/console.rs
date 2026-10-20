//! Console settings stay in the User config; serving never writes them.
use anyhow::{Result, ensure};
use serde::Deserialize;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    pub address: String,
    pub port: u16,
    pub allowed_emails: Vec<String>,
    pub other_consoles: Vec<String>,
    pub theme: String,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            address: String::new(),
            port: 8711,
            allowed_emails: Vec::new(),
            other_consoles: Vec::new(),
            theme: "focus-auto".into(),
        }
    }
}

impl ConsoleSettings {
    /// The HTTPS site's DNS name, accepted with or without `https://`.
    pub fn hostname(&self) -> Result<&str> {
        let host = self
            .address
            .strip_prefix("https://")
            .unwrap_or(&self.address);
        let host = host.strip_suffix('/').unwrap_or(host);
        ensure!(
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }),
            "console.address must be an HTTPS DNS name, such as https://console.example.com, without a port or path"
        );
        Ok(host)
    }

    pub(super) fn validate(&self) -> Result<()> {
        if !self.address.is_empty() {
            self.hostname()?;
        }
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
