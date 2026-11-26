//! Fetch each owning machine's Inbox once, preserving its grouping and facts.
//! The Console poller can reuse this blocking fetch-and-merge path off its executor.
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config::{UserConfig, console::fleet::Entry};
use crate::resend_key::Credentials;

pub(super) const VERSION: u32 = 2;
const TIMEOUT: Duration = Duration::from_secs(3);
const NESTED_COMMANDS: [&str; 3] = ["children", "dispatched", "collapsed"];

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Document {
    pub version: u32,
    pub hostname: String,
    pub active_runs: usize,
    // Keep the owning machine's Command-specific metadata intact. Fleet merging
    // never re-evaluates its GitHub facts, collapse decisions or account ownership.
    pub(super) commands: Vec<Observation>,
    #[serde(default)]
    pub machines: Vec<Machine>,
    #[serde(flatten)]
    other: Map<String, Value>,
}

impl Document {
    pub(crate) fn omit_command(&mut self, selector: &str) {
        fn keep(value: &mut Value, selector: &str) -> bool {
            if super::document_selector(value) == selector {
                return false;
            }
            for field in NESTED_COMMANDS {
                if let Some(children) = value.get_mut(field).and_then(Value::as_array_mut) {
                    children.retain_mut(|child| keep(child, selector));
                }
            }
            true
        }
        self.commands
            .retain_mut(|command| keep(&mut command.value, selector));
    }

    pub(super) fn github_answer_is_stale(&self) -> bool {
        super::stale_github_answer(
            self.other
                .get("github_answer_age_seconds")
                .and_then(Value::as_u64),
            self.other
                .get("github_failure")
                .is_some_and(Value::is_string),
        )
    }

    fn tag_commands(&mut self, name: Option<&str>) -> Result<()> {
        let stale_github = self.github_answer_is_stale();
        let name = name.unwrap_or(&self.hostname);
        for command in &mut self.commands {
            command.stale_github = stale_github;
            tag(&mut command.value, &self.hostname, name)?;
        }
        Ok(())
    }
}

/// Carry the owning machine's freshness through the Fleet merge in memory.
/// The wire representation remains the original Command object.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Observation {
    pub value: Value,
    #[serde(skip)]
    stale_github: bool,
}

impl Observation {
    pub fn text_row(&self) -> Result<super::text::Row> {
        super::text::Row::read(&self.value, self.stale_github)
    }
}

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

pub(super) fn local(value: Value, now: DateTime<Utc>) -> Result<Document> {
    let mut document: Document = serde_json::from_value(value)?;
    document.tag_commands(None)?;
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

fn tag(row: &mut Value, machine: &str, name: &str) -> Result<()> {
    let fields = row.as_object_mut().context("invalid Command")?;
    fields.insert("machine".into(), machine.into());
    fields.insert("machine_name".into(), name.into());
    for field in NESTED_COMMANDS {
        if let Some(nested) = fields.get_mut(field) {
            for child in nested.as_array_mut().context("invalid Command tree")? {
                tag(child, machine, name)?;
            }
        }
    }
    Ok(())
}

/// `None` and the local hostname both mean the entire Fleet. An explicit other
/// name selects only that entry. Link-only entries never receive a request.
pub(crate) fn fetch_and_merge(
    config: &UserConfig,
    mut local: Document,
    machine: Option<&str>,
    all: bool,
) -> Result<Document> {
    let selected = machine.filter(|name| *name != local.hostname);
    if selected.is_none() && config.console.fleet.is_empty() {
        return Ok(local);
    }
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
    // Both local and received rows are validated presentations. Sorting uses the
    // same key as the owning Inbox, without re-evaluating groups or collapsing
    // independent Commands from different machines.
    let mut rows = local
        .commands
        .into_iter()
        .map(|command| Ok((command.text_row()?.order(), command)))
        .collect::<Result<Vec<_>>>()?;
    rows.sort_by_key(|(order, _)| *order);
    local.commands = rows.into_iter().map(|(_, value)| value).collect();
    Ok(local)
}

pub(crate) fn fetch(
    client: &reqwest::blocking::Client,
    entry: &Entry,
    token: &str,
    all: bool,
) -> std::result::Result<Document, (Reachability, String)> {
    fetch_for_person(client, entry, token, all, None)
}

pub(crate) fn fetch_for_person(
    client: &reqwest::blocking::Client,
    entry: &Entry,
    token: &str,
    all: bool,
    person: Option<&str>,
) -> std::result::Result<Document, (Reachability, String)> {
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
    let mut document: Document = response.json().map_err(|error| {
        if error.is_timeout() || error.is_body() {
            (
                Unreachable,
                "Inbox body failed or timed out (3 seconds)".into(),
            )
        } else {
            (NeedsUpdate, "incompatible Inbox response".into())
        }
    })?;
    document
        .tag_commands(Some(&entry.name))
        .map_err(|_| (NeedsUpdate, "invalid Inbox Command tree".into()))?;
    if document.version != VERSION
        || document
            .commands
            .iter()
            .any(|command| command.text_row().is_err())
    {
        return Err((NeedsUpdate, "unsupported Inbox version or rows".into()));
    }
    Ok(document)
}
