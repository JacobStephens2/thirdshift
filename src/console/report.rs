//! Outbound reporting (CLI-only): pairing the Consoles this machine pushes
//! its own Run metrics summary and Plan usage to, and the push itself, by
//! hand with `metrics --push` or `plan-usage --push`, or after each Command.
//! A sleeping or NATted machine reaches an always-on Console this way
//! without being reached inbound (#2036 Q4.2). No token enters arguments,
//! output or User config.
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use super::Scope;
use super::fleet::WhoamiIdentity;
use crate::args::{MetricsArgs, MetricsGrain};
use crate::config::{UserConfig, console::report::Destination};
use crate::logs::metrics::fleet::MetricsReport;
use crate::plan_usage::fleet::Report as PlanUsageReport;
use crate::resend_key::Credentials;

/// How long a push after a Command may wait on each Console.
const AFTER_COMMAND: Duration = Duration::from_secs(5);
/// How long `metrics --push` may wait on each Console.
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
                self::address(address)?;
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

/// The Fleet name the receiving Console issued `token` to, once it shows the
/// token carries `report` alone.
fn holder(address: &str, token: &str) -> Result<String> {
    let identity = super::fleet::whoami(address, token, "Report destination")?;
    ensure!(
        identity.scopes == [Scope::Report],
        "the token must carry the report scope alone; create it with thirdshift console token create <this machine> --scopes report"
    );
    match identity.identity {
        WhoamiIdentity::Token { holder } => Ok(holder),
        WhoamiIdentity::Other => bail!("Console did not name the report token's holder"),
    }
}

/// A report destination's address, checked as a Fleet pairing's is before a
/// saved token is sent to it.
pub(crate) fn address(address: &str) -> Result<reqwest::Url> {
    super::fleet::token_address(address, "Report destination")
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

/// One thing this machine reports: the route on a Console it goes to, and
/// its body, whose `machine` names the holder per destination.
struct Outgoing {
    route: &'static str,
    body: serde_json::Value,
}

impl Outgoing {
    fn metrics(report: &MetricsReport) -> Result<Self> {
        Ok(Self {
            route: "/api/v1/metrics/report",
            body: serde_json::to_value(report)?,
        })
    }

    fn plan_usage(report: &PlanUsageReport) -> Result<Self> {
        Ok(Self {
            route: "/api/v1/plan-usage/report",
            body: serde_json::to_value(report)?,
        })
    }

    /// Name it for `holder`, the machine the destination issued its token to.
    fn name(&mut self, holder: &str) {
        self.body["machine"] = holder.into();
    }
}

/// The report destinations, or the one `only` names.
fn destinations<'a>(config: &'a UserConfig, only: Option<&str>) -> Result<Vec<&'a Destination>> {
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
    Ok(destinations)
}

/// Push each of `reports`, in order, to each of `destinations`. Each
/// destination's outcome: its first refusal, or success.
fn push(
    destinations: Vec<&Destination>,
    mut reports: Vec<Outgoing>,
    timeout: Duration,
) -> Result<Vec<(String, Result<()>)>> {
    if destinations.is_empty() {
        return Ok(Vec::new());
    }
    let credentials = Credentials::read()?;
    let client = client(timeout)?;
    Ok(destinations
        .into_iter()
        .map(|destination| {
            let sent = reports.iter_mut().try_for_each(|report| {
                report.name(&destination.holder);
                send(&client, &credentials, destination, report)
            });
            (destination.name.clone(), sent)
        })
        .collect())
}

fn send(
    client: &reqwest::blocking::Client,
    credentials: &Credentials,
    destination: &Destination,
    report: &Outgoing,
) -> Result<()> {
    // Revalidate a hand-edited address before transmitting the saved token.
    address(&destination.address)?;
    let token = credentials
        .report_token(&destination.name)
        .with_context(|| format!("no report token saved for {}", destination.name))?;
    let response = client
        .post(format!(
            "{}{}",
            destination.address.trim_end_matches('/'),
            report.route
        ))
        .bearer_auth(token)
        .json(&report.body)
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

/// Each destination's outcome as one line, failing unless every push
/// succeeded.
fn by_hand(pushed: Vec<(String, Result<()>)>) -> Result<String> {
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

/// `metrics --push [<destination>]`: fails unless every push succeeds.
pub(crate) fn push_by_hand(asked: &MetricsArgs) -> Result<String> {
    let config = UserConfig::load()?;
    let destinations = destinations(&config, asked.push_to.as_deref())?;
    if destinations.is_empty() {
        return by_hand(Vec::new());
    }
    let report = Outgoing::metrics(&MetricsReport::local(&config, asked)?)?;
    by_hand(push(destinations, vec![report], BY_HAND)?)
}

/// `plan-usage --push [<destination>]`: this machine's saved Plan usage,
/// failing unless every push succeeds.
pub(crate) fn push_plan_usage_by_hand(only: Option<&str>) -> Result<String> {
    let config = UserConfig::load()?;
    let destinations = destinations(&config, only)?;
    if destinations.is_empty() {
        return by_hand(Vec::new());
    }
    let report = Outgoing::plan_usage(&PlanUsageReport::local(&config)?)?;
    by_hand(push(destinations, vec![report], BY_HAND)?)
}

/// After a Command: push every report destination the whole history by day,
/// which stands in for each window's view, then the default view, then this
/// machine's saved Plan usage, each send bounded by a short timeout. Never
/// fails the Command: an offline or refusing Console only earns a warning.
/// Nothing is pushed between Commands.
pub(crate) fn after_command() {
    let Ok(config) = UserConfig::load() else {
        return;
    };
    if config.console.report.is_empty() {
        return;
    }
    let daily = MetricsArgs {
        grain: Some(MetricsGrain::Day),
        ..MetricsArgs::default()
    };
    let reports = [daily, MetricsArgs::default()]
        .iter()
        .map(|asked| Outgoing::metrics(&MetricsReport::local(&config, asked)?))
        .chain([PlanUsageReport::local(&config).and_then(|report| Outgoing::plan_usage(&report))])
        .collect::<Result<Vec<_>>>();
    let pushed =
        reports.and_then(|reports| push(destinations(&config, None)?, reports, AFTER_COMMAND));
    match pushed {
        Ok(pushed) => {
            for (name, sent) in pushed {
                if let Err(error) = sent {
                    crate::progress::step(format_args!(
                        "warning: metrics and Plan usage not reported to {name}: {error:#}"
                    ));
                }
            }
        }
        Err(error) => crate::progress::step(format_args!(
            "warning: metrics and Plan usage not reported: {error:#}"
        )),
    }
}
