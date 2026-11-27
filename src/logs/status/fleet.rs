//! Fetch each owning machine's Inbox once, preserving its grouping and facts.
//! The Console poller can reuse this blocking fetch-and-merge path off its executor.
use std::time::Duration;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{UserConfig, console::fleet::Entry};
use crate::resend_key::Credentials;

use super::rows::{ConsoleDocument, Document, WireDocument};

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Machine {
    pub name: String,
    pub hostname: Option<String>,
    pub fetched_at: DateTime<Utc>,
    pub reachability: Reachability,
    pub active_runs: usize,
    pub error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Reachability {
    Ok,
    Unreachable,
    Refused,
    #[serde(rename = "needs update")]
    NeedsUpdate,
}

pub(super) fn local(value: Value, now: DateTime<Utc>) -> Result<WireDocument> {
    let mut document: WireDocument = serde_json::from_value(value)?;
    document.tag(None)?;
    let hostname = &document.hostname;
    document.machines = vec![Machine {
        name: hostname.clone(),
        hostname: Some(hostname.clone()),
        fetched_at: now,
        reachability: Reachability::Ok,
        active_runs: document.active_runs,
        error: None,
    }];
    Ok(document)
}

/// `None` and the local hostname both mean the entire Fleet. An explicit other
/// name selects only that entry. Link-only entries never receive a request.
pub(super) fn fetch_and_merge(
    config: &UserConfig,
    mut local: WireDocument,
    machine: Option<&str>,
    all: bool,
) -> Result<Document> {
    let selected = machine.filter(|name| *name != local.hostname);
    let credentials = Credentials::read()?;
    if let Some(name) = selected {
        ensure!(
            config
                .console
                .fleet
                .iter()
                .any(|entry| entry.name == name && credentials.has_fleet_token(name)),
            "unknown or link-only Fleet machine: {name}"
        );
        local.commands.clear();
        local.machines.clear();
        local.active_runs = 0;
        local
            .other
            .insert("idle_passes".into(), Value::Array(vec![]));
        local
            .other
            .insert("pass_starts".into(), Value::Array(vec![]));
    }
    let mut local = local.check_cli()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    for entry in &config.console.fleet {
        if selected.is_some_and(|name| name != entry.name) {
            continue;
        }
        let Some(token) = credentials.fleet_token(&entry.name) else {
            continue;
        };
        let mut machine = Machine {
            name: entry.name.clone(),
            hostname: entry.hostname.clone(),
            fetched_at: Utc::now(),
            reachability: Reachability::Ok,
            active_runs: 0,
            error: None,
        };
        match fetch(&client, entry, token, all) {
            Ok(remote) => {
                machine.hostname = Some(remote.hostname);
                machine.active_runs = remote.active_runs;
                local.commands.extend(remote.commands);
                local.active_runs += machine.active_runs;
            }
            Err((reachability, error)) => {
                machine.reachability = reachability;
                machine.error = Some(error);
            }
        }
        machine.fetched_at = Utc::now();
        local.machines.push(machine);
    }
    local.commands.sort_by_cached_key(super::rows::Row::order);
    Ok(local)
}

fn fetch(
    client: &reqwest::blocking::Client,
    entry: &Entry,
    token: &str,
    all: bool,
) -> std::result::Result<Document, (Reachability, String)> {
    fetch_wire(client, entry, token, all, None)?
        .admit_cli(&entry.name)
        .map_err(|error| (Reachability::NeedsUpdate, error.message().into()))
}

pub(crate) fn fetch_for_person(
    client: &reqwest::blocking::Client,
    entry: &Entry,
    token: &str,
    all: bool,
    person: Option<&str>,
) -> std::result::Result<ConsoleDocument, (Reachability, String)> {
    fetch_wire(client, entry, token, all, person)?
        .admit_console(&entry.name)
        .map_err(|error| (Reachability::NeedsUpdate, error.message().into()))
}

fn fetch_wire(
    client: &reqwest::blocking::Client,
    entry: &Entry,
    token: &str,
    all: bool,
    person: Option<&str>,
) -> std::result::Result<WireDocument, (Reachability, String)> {
    use Reachability::{NeedsUpdate, Refused, Unreachable};
    // Recheck edited config before transmitting the saved bearer token.
    crate::console::fleet::pairing_address(&entry.address)
        .map_err(|_| (Refused, "unsafe Fleet address".into()))?;
    let mut request = client
        .get(format!(
            "{}/api/v1/status?all={all}",
            entry.address.trim_end_matches('/')
        ))
        .bearer_auth(token);
    if let Some(person) = person {
        request = request.header("x-thirdshift-person", person);
    }
    let response = request.send().map_err(|_| {
        (
            Unreachable,
            "Inbox request failed or timed out (3 seconds)".into(),
        )
    })?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err((NeedsUpdate, "Console has no /api/v1/status".into()));
    }
    if !status.is_success() {
        let state = if status.is_client_error() || status.is_redirection() {
            Refused
        } else {
            Unreachable
        };
        return Err((state, format!("Inbox request returned HTTP {status}")));
    }
    response.json().map_err(|error| {
        if error.is_timeout() || error.is_body() {
            (
                Unreachable,
                "Inbox body failed or timed out (3 seconds)".into(),
            )
        } else {
            (NeedsUpdate, "incompatible Inbox response".into())
        }
    })
}
