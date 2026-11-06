//! Request-owned Fleet details. Remote paths are identifiers, never local files.
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::config::{UserConfig, console::fleet::Entry};
use crate::resend_key::Credentials;

pub(crate) struct Remote {
    pub entry: Entry,
    client: reqwest::blocking::Client,
    token: String,
    person: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Refusal {
    pub status: reqwest::StatusCode,
    pub message: String,
    pub address: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Refusal {}

impl Remote {
    pub fn cli(machine: &str, route: &str) -> Result<String> {
        let config = UserConfig::load()?;
        let remote =
            Self::select(&config, machine, None)?.context("expected a remote Fleet machine")?;
        Ok(serde_json::to_string_pretty(&remote.get(route)?)?)
    }

    pub fn select(config: &UserConfig, name: &str, person: Option<String>) -> Result<Option<Self>> {
        if name.is_empty() || crate::host::name().is_some_and(|hostname| hostname == name) {
            return Ok(None);
        }
        let entry = config
            .console
            .fleet
            .iter()
            .find(|entry| entry.name == name)
            .with_context(|| format!("unknown Fleet machine: {name}"))?
            .clone();
        // Revalidate hand-edited addresses before sending a saved credential.
        super::pairing_address(&entry.address)?;
        let token = Credentials::read()?
            .fleet_token(name)
            .with_context(|| format!("link-only Fleet machine: {name}"))?
            .to_owned();
        Ok(Some(Self {
            entry,
            token,
            person,
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        }))
    }

    pub fn get(&self, route: &str) -> Result<Value> {
        ensure!(
            route.starts_with('/') && !route.starts_with("//"),
            "invalid details route"
        );
        let mut request = self
            .client
            .get(format!(
                "{}/api/v1{route}",
                self.entry.address.trim_end_matches('/')
            ))
            .bearer_auth(&self.token);
        if let Some(person) = &self.person {
            request = request.header(super::super::auth::PERSON_HEADER, person);
        }
        let response = request.send().map_err(|_| {
            self.refusal(
                reqwest::StatusCode::BAD_GATEWAY,
                format!(
                    "Details request to {} failed or timed out (3 seconds)",
                    self.entry.name
                ),
            )
        })?;
        let status = response.status();
        if !status.is_success() {
            let error = response
                .json::<Value>()
                .ok()
                .and_then(|value| value["error"].as_str().map(str::to_owned))
                .unwrap_or_default();
            let message = if status == reqwest::StatusCode::FORBIDDEN && error.contains("security")
            {
                format!("needs the `security` scope on `{}`", self.entry.name)
            } else {
                format!(
                    "{} refused the details request (HTTP {status}): {error}",
                    self.entry.name
                )
            };
            return Err(self.refusal(status, message).into());
        }
        response.json().map_err(|_| {
            self.refusal(
                reqwest::StatusCode::BAD_GATEWAY,
                format!("{} returned incompatible details", self.entry.name),
            )
            .into()
        })
    }

    fn refusal(&self, status: reqwest::StatusCode, message: String) -> Refusal {
        Refusal {
            status,
            message,
            address: self.entry.address.clone(),
        }
    }

    pub fn url(&self, route: &str) -> String {
        crate::logs::status::inspect::machine_url(route, &self.entry.name)
    }

    pub fn command_url(
        &self,
        route: &str,
        command: &str,
        session: Option<&std::path::Path>,
    ) -> String {
        self.url(&crate::logs::status::inspect::url(route, command, session))
    }
}
