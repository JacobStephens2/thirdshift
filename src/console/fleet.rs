//! CLI-only Fleet pairing. No token enters arguments, output or User config.
use std::fs::{self, OpenOptions};
use std::io::{IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::Scope;
use crate::config::{
    self, UserConfig,
    console::fleet::{self, Entry},
};
use crate::resend_key::Credentials;

pub(crate) enum Action {
    Add { name: String, address: String },
    List { json: bool },
    Remove(String),
}

impl Action {
    pub(crate) fn parse(args: &[String]) -> Result<Self> {
        let usage = "usage: thirdshift console fleet add <name> <address> (token on stdin) | list [--json] | remove <name>";
        match args {
            [action, name, address] if action == "add" => {
                fleet::validate_name(name)?;
                pairing_address(address)?;
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
                // Reject invalid local files before prompting or contacting a machine.
                UserConfig::load()?;
                Credentials::read()?;
                let token = read_token()?;
                let entry = check(name, address, &token)?;
                let _lock = lock()?;
                let credentials = Credentials::read()?;
                fleet::pair(&entry)?;
                credentials.save_fleet_token(&entry.name, Some(&token))?;
                Ok(format!("Paired {} ({})", entry.name, entry.address))
            }
            Self::Remove(name) => {
                let _lock = lock()?;
                let credentials = Credentials::read()?;
                fleet::remove(&name)?;
                credentials.save_fleet_token(&name, None)?;
                Ok(format!("Removed {name}"))
            }
        }
    }
}

fn lock() -> Result<std::fs::File> {
    let root = config::home()?.join(".thirdshift");
    fs::create_dir_all(&root)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(root.join("fleet.lock"))?;
    lock.lock()?;
    Ok(lock)
}

fn read_token() -> Result<String> {
    crate::interrupt::install()?;
    let terminal = std::io::stdin().is_terminal();
    let _echo = terminal.then(crate::terminal::EchoOff::new).transpose()?;
    if terminal {
        eprint!("Fleet token: ");
        std::io::stderr().flush()?;
    }
    let token = crate::terminal::line()?.context("Fleet token input ended")?;
    if terminal {
        eprintln!();
    }
    let token = token.trim().to_owned();
    ensure!(!token.is_empty(), "Fleet token is empty");
    Ok(token)
}

fn pairing_address(address: &str) -> Result<reqwest::Url> {
    let url = fleet::link_address(address)?;
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Fleet pairing addresses must have no credentials, query or fragment"
    );
    ensure!(
        url.scheme() == "https" || url.host_str() == Some("127.0.0.1"),
        "Fleet pairing requires HTTPS except for 127.0.0.1"
    );
    Ok(url)
}

#[derive(Deserialize)]
struct Whoami {
    version: u8,
    api_version: String,
    hostname: String,
    thirdshift_version: String,
    scopes: Vec<Scope>,
}

fn check(name: String, address: String, token: &str) -> Result<Entry> {
    pairing_address(&address)?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?
        .get(format!("{}/api/v1/whoami", address.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .map_err(|_| anyhow::anyhow!("Fleet machine unreachable: {address}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("Fleet machine too old: {address} has no /api/v1/whoami");
    }
    ensure!(
        status.is_success(),
        "Fleet pairing refused by {address} (HTTP {status})"
    );
    let identity: Whoami = response.json().map_err(|_| {
        anyhow::anyhow!("Fleet machine too old or incompatible: invalid whoami response")
    })?;
    ensure!(
        identity.version == 1 && identity.api_version == "v1",
        "Fleet machine too old or incompatible: unsupported whoami version"
    );
    Ok(Entry {
        name,
        address,
        hostname: Some(identity.hostname),
        thirdshift_version: Some(identity.thirdshift_version),
        scopes: identity.scopes,
        last_contact: Some(Utc::now()),
    })
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    entry: Entry,
    live: bool,
}

fn list(json: bool) -> Result<String> {
    let settings = UserConfig::load()?;
    let credentials = Credentials::read()?;
    let entries: Vec<_> = settings
        .console
        .fleet
        .into_iter()
        .map(|entry| {
            let live = credentials.has_fleet_token(&entry.name);
            Listed { entry, live }
        })
        .collect();
    if json {
        return Ok(serde_json::to_string(&entries)?);
    }
    Ok(entries
        .iter()
        .map(|listed| {
            let entry = &listed.entry;
            format!(
                "{}\t{}\t{}\tscopes={}\tthirdshift={}\tlast contact={}",
                entry.name,
                entry.address,
                if listed.live { "live" } else { "link-only" },
                entry
                    .scopes
                    .iter()
                    .map(|scope| scope.name())
                    .collect::<Vec<_>>()
                    .join(","),
                entry.thirdshift_version.as_deref().unwrap_or("unknown"),
                entry
                    .last_contact
                    .map(|time| time.to_rfc3339())
                    .unwrap_or_else(|| "never".into())
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
