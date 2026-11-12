//! Outbound metrics reporting (CLI-only): pairing the Consoles this machine
//! pushes its own Run metrics summary to, and the push itself, by hand with
//! `metrics --push` or after each Command. A sleeping or NATted machine
//! reaches an always-on Console this way without being reached inbound
//! (#2036 Q4.2). No token enters arguments, output or User config.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use super::Scope;
use crate::args::MetricsArgs;
use crate::config::{UserConfig, console::report::Destination};
use crate::logs::metrics::fleet::Report;
use crate::resend_key::Credentials;

/// How long a push after a Command may wait on each Console.
const AFTER_COMMAND: Duration = Duration::from_secs(5);
/// How long `metrics --push` and pairing may wait on each Console.
const BY_HAND: Duration = Duration::from_secs(10);

pub(crate) enum Action {
    Add { name: String, address: String },
    List { json: bool },
    Remove(String),
}

impl Action {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let usage = "usage: thirdshift console report add <name> <address> (report token on stdin) | list [--json] | remove <name>";
        match args {
            [action, name, address] if action == "add" => {
                crate::config::console::fleet::validate_name(name)?;
                super::fleet::pairing_address(address)?;
                Ok(Self::Add {
                    name: name.clone(),
                    address: address.clone(),
                })
            }
            [action] if action == "list" => Ok(Self::List { json: false }),
            [action, flag] if action == "list" && flag == "--json" => Ok(Self::List { json: true }),
            [action, name] if action == "remove" => Ok(Self::Remove(name.clone())),
            _ => bail!(usage),
        }
    }

    pub(crate) fn run(self) -> Result<String> {
        match self {
            Self::List { json } => list(json),
            Self::Add { name, address } => {
                // Reject invalid local files before prompting or contacting a Console.
                UserConfig::load()?;
                Credentials::read()?;
                let token = super::fleet::read_secret("Report token")?;
                let holder = holder(&address, &token)?;
                let destination = Destination {
                    name,
                    address,
                    holder,
                };
                let _lock = super::fleet::lock()?;
                let credentials = Credentials::read()?;
                crate::config::console::report::pair(&destination)?;
                credentials.save_report_token(&destination.name, Some(&token))?;
                Ok(format!(
                    "Reporting to {} ({}) as {}",
                    destination.name, destination.address, destination.holder
                ))
            }
            Self::Remove(name) => {
                let _lock = super::fleet::lock()?;
                let credentials = Credentials::read()?;
                ensure!(
                    UserConfig::load()?
                        .console
                        .report
                        .iter()
                        .any(|destination| destination.name == name),
                    "unknown report destination: {name}"
                );
                crate::config::console::report::remove(&name)?;
                credentials.save_report_token(&name, None)?;
                Ok(format!("Removed {name}"))
            }
        }
    }
}

#[derive(Deserialize)]
struct Whoami {
    version: u8,
    api_version: String,
    identity: Identity,
    scopes: Vec<Scope>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Identity {
    Token {
        holder: String,
    },
    #[serde(other)]
    Other,
}

/// The Fleet name the receiving Console issued `token` to, once it shows the
/// token carries `report` alone.
fn holder(address: &str, token: &str) -> Result<String> {
    let response = client(BY_HAND)?
        .get(format!("{}/api/v1/whoami", address.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .map_err(|_| anyhow::anyhow!("Console unreachable: {address}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("Console too old: {address} has no /api/v1/whoami");
    }
    ensure!(
        status.is_success(),
        "report token refused by {address} (HTTP {status})"
    );
    let identity: Whoami = response
        .json()
        .map_err(|_| anyhow::anyhow!("Console too old or incompatible: invalid whoami response"))?;
    ensure!(
        identity.version == 1 && identity.api_version == "v1",
        "Console too old or incompatible: unsupported whoami version"
    );
    ensure!(
        identity.scopes == [Scope::Report],
        "the token must carry the report scope alone; create it with thirdshift console token create <this machine> --scopes report"
    );
    match identity.identity {
        Identity::Token { holder } => Ok(holder),
        Identity::Other => bail!("Console did not name the report token's holder"),
    }
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

#[derive(Serialize)]
struct Listed<'a> {
    #[serde(flatten)]
    destination: &'a Destination,
    token: bool,
}

fn list(json: bool) -> Result<String> {
    let config = UserConfig::load()?;
    let credentials = Credentials::read()?;
    let listed: Vec<_> = config
        .console
        .report
        .iter()
        .map(|destination| Listed {
            destination,
            token: credentials.report_token(&destination.name).is_some(),
        })
        .collect();
    if json {
        return Ok(serde_json::to_string(&listed)?);
    }
    Ok(listed
        .iter()
        .map(|listed| {
            format!(
                "{}\t{}\tas {}\t{}",
                listed.destination.name,
                listed.destination.address,
                listed.destination.holder,
                if listed.token {
                    "token saved"
                } else {
                    "no token"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Push this machine's summary of the view `asked` names to each report
/// destination, or to the one `only` names. Each destination's outcome.
fn push(
    config: &UserConfig,
    asked: &MetricsArgs,
    only: Option<&str>,
    timeout: Duration,
) -> Result<Vec<(String, Result<()>)>> {
    let destinations: Vec<_> = config
        .console
        .report
        .iter()
        .filter(|destination| only.is_none_or(|name| name == destination.name))
        .collect();
    if let Some(name) = only {
        ensure!(
            !destinations.is_empty(),
            "unknown report destination: {name}"
        );
    }
    if destinations.is_empty() {
        return Ok(Vec::new());
    }
    let credentials = Credentials::read()?;
    let mut report = Report::local(config, asked)?;
    let client = client(timeout)?;
    Ok(destinations
        .into_iter()
        .map(|destination| {
            report.machine = destination.holder.clone();
            let sent = send(&client, &credentials, destination, &report);
            (destination.name.clone(), sent)
        })
        .collect())
}

fn send(
    client: &reqwest::blocking::Client,
    credentials: &Credentials,
    destination: &Destination,
    report: &Report,
) -> Result<()> {
    // Revalidate a hand-edited address before transmitting the saved token.
    super::fleet::pairing_address(&destination.address)?;
    let token = credentials
        .report_token(&destination.name)
        .with_context(|| format!("no report token saved for {}", destination.name))?;
    let response = client
        .post(format!(
            "{}/api/v1/metrics/report",
            destination.address.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .json(report)
        .send()
        .map_err(|error| {
            anyhow::anyhow!(
                "{} unreachable or timed out: {}",
                destination.address,
                error.without_url()
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let message = response
            .json::<serde_json::Value>()
            .ok()
            .and_then(|body| body["error"].as_str().map(str::to_owned))
            .unwrap_or_default();
        bail!(
            "{} refused the report (HTTP {status}){}",
            destination.address,
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        );
    }
    Ok(())
}

/// `metrics --push [<destination>]`: fails unless every push succeeds.
pub(crate) fn push_by_hand(asked: &MetricsArgs) -> Result<String> {
    let config = UserConfig::load()?;
    let only = asked.push_to.as_deref();
    let pushed = push(&config, asked, only, BY_HAND)?;
    ensure!(
        !pushed.is_empty(),
        "no report destinations; pair one with thirdshift console report add <name> <address>"
    );
    let mut lines = Vec::new();
    let mut failed = 0;
    for (name, sent) in pushed {
        match sent {
            Ok(()) => lines.push(format!("Reported to {name}")),
            Err(error) => {
                failed += 1;
                lines.push(format!("Not reported to {name}: {error:#}"));
            }
        }
    }
    let text = lines.join("\n");
    ensure!(failed == 0, "{text}");
    Ok(text)
}

/// After a Command: push the default view to every report destination,
/// each bounded by a short timeout. Never fails the Command: an offline or
/// refusing Console only earns a warning.
pub(crate) fn after_command() {
    let Ok(config) = UserConfig::load() else {
        return;
    };
    if config.console.report.is_empty() {
        return;
    }
    match push(&config, &MetricsArgs::default(), None, AFTER_COMMAND) {
        Ok(pushed) => {
            for (name, sent) in pushed {
                if let Err(error) = sent {
                    crate::progress::step(format_args!(
                        "warning: metrics not reported to {name}: {error:#}"
                    ));
                }
            }
        }
        Err(error) => {
            crate::progress::step(format_args!("warning: metrics not reported: {error:#}"))
        }
    }
}
