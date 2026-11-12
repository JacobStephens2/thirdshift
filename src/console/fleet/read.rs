//! Request-owned Fleet details. Remote paths are identifiers, never local files.
use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde_json::Value;

use crate::config::{UserConfig, console::fleet::Entry};

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

/// Why a Fleet read failed, as a Fleet report names it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Unavailable,
    Refused,
    Incompatible,
}

impl Failure {
    pub(crate) fn of(error: &anyhow::Error) -> Self {
        if let Some(refusal) = error.downcast_ref::<Refusal>() {
            match refusal.status.as_u16() {
                401 | 403 => Self::Refused,
                404 => Self::Incompatible,
                _ if refusal.message.contains("incompatible") => Self::Incompatible,
                _ => Self::Unavailable,
            }
        } else if error.to_string().contains("incompatible") {
            Self::Incompatible
        } else {
            Self::Unavailable
        }
    }
}

pub(crate) fn owning_url(entry: &Entry, route: &str) -> String {
    format!("{}{}", entry.address.trim_end_matches('/'), route)
}

impl Remote {
    pub fn cli(machine: &str, route: &str) -> Result<String> {
        let config = UserConfig::load()?;
        let remote =
            Self::select(&config, machine, None)?.context("expected a remote Fleet machine")?;
        Ok(serde_json::to_string_pretty(&remote.get(route)?)?)
    }

    pub fn select(config: &UserConfig, name: &str, person: Option<String>) -> Result<Option<Self>> {
        if name.is_empty() {
            return Ok(None);
        }
        let super::PairedMachine { entry, token } = super::PairedMachine::load(config, name)?;
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
        self.read(route, None)
    }

    pub(in crate::console) fn identity(&self) -> Result<super::Whoami> {
        let identity: super::Whoami = serde_json::from_value(self.get("/whoami")?)
            .context("incompatible owning-machine identity")?;
        ensure!(
            identity.version == 1 && identity.api_version == "v1",
            "unsupported owning-machine API version"
        );
        Ok(identity)
    }

    pub fn post_json(&self, route: &str, value: &Value) -> Result<Value> {
        self.mutate(self.client.post(self.api_url(route)?).json(value))
    }

    pub fn post_form(&self, route: &str, fields: &[(&str, &str)]) -> Result<Value> {
        self.mutate(self.client.post(self.api_url(route)?).form(fields))
    }

    fn api_url(&self, route: &str) -> Result<String> {
        ensure!(
            route.starts_with('/') && !route.starts_with("//"),
            "invalid Fleet route"
        );
        Ok(format!(
            "{}/api/v1{route}",
            self.entry.address.trim_end_matches('/')
        ))
    }

    fn mutate(&self, request: reqwest::blocking::RequestBuilder) -> Result<Value> {
        let mut request = request
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(300));
        if let Some(person) = &self.person {
            request = request.header(super::super::auth::PERSON_HEADER, person);
        }
        let response = request.send().map_err(|_| {
            self.refusal(
                reqwest::StatusCode::BAD_GATEWAY,
                format!(
                    "{} request failed or timed out; check its Inbox before retrying",
                    self.entry.name
                ),
            )
        })?;
        let status = response.status();
        let value: Value = response.json().context("incompatible Fleet response")?;
        ensure!(
            status.is_success(),
            self.refusal(
                status,
                format!(
                    "{} refused the request (HTTP {status}): {}",
                    self.entry.name, value
                )
            )
        );
        Ok(value)
    }

    pub fn preview(&self, command: &str) -> Result<String> {
        let route = crate::logs::status::inspect::url("/log", command, None);
        // 4096 raw bytes can expand sixfold when JSON escapes them. Refuse an
        // older endpoint that ignores tail instead of downloading a full log.
        let value = self.read(&format!("{route}&tail=true"), Some(32 * 1024))?;
        value["contents"]
            .as_str()
            .map(str::to_owned)
            .context("incompatible remote preview")
    }

    fn read(&self, route: &str, limit: Option<usize>) -> Result<Value> {
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
        if let Some(limit) = limit {
            let mut bytes = Vec::new();
            response
                .take((limit + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    self.refusal(
                        reqwest::StatusCode::BAD_GATEWAY,
                        "Remote preview body failed".into(),
                    )
                })?;
            ensure!(
                bytes.len() <= limit,
                "owning Console needs an update for bounded previews"
            );
            return serde_json::from_slice(&bytes).context("incompatible remote preview");
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

    /// A paired machine for rendering tests. It never sends the stored token.
    #[cfg(test)]
    pub(crate) fn for_render_test(name: &str, address: &str) -> Self {
        Self {
            entry: Entry::link(name.to_string(), address.to_string()),
            client: reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("test client"),
            token: String::new(),
            person: None,
        }
    }
}
