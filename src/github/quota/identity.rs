//! Cache the authenticated login without keeping credentials. A changed
//! environment token or gh account configuration requires a new observation.
use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::store::{Account, Store};
use crate::github::GitHub;

pub(super) fn host(args: &[&str]) -> String {
    for arg in args {
        if let Some(host) = arg.strip_prefix("--hostname=") {
            return host.to_ascii_lowercase();
        }
        if let Some(host) = url_host(arg) {
            return host;
        }
    }
    for pair in args.windows(2) {
        if matches!(pair[0], "--hostname" | "-h") {
            return pair[1].to_ascii_lowercase();
        }
        if matches!(pair[0], "--repo" | "-R")
            && let [hostname, _, _] = pair[1].split('/').collect::<Vec<_>>().as_slice()
        {
            return hostname.to_ascii_lowercase();
        }
    }
    std::env::var("GH_HOST")
        .ok()
        .filter(|host| !host.is_empty())
        .unwrap_or_else(|| "github.com".into())
        .to_ascii_lowercase()
}

fn url_host(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    let host = url.host_str()?;
    Some(if host == "api.github.com" {
        "github.com".into()
    } else {
        host.to_ascii_lowercase()
    })
}

fn credential() -> Result<String> {
    let mut digest = Sha256::new();
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        digest.update(name);
        digest.update([0]);
        if let Ok(value) = std::env::var(name) {
            digest.update(value);
        }
        digest.update([0]);
    }
    let dir = std::env::var_os("GH_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|dir| std::path::PathBuf::from(dir).join("gh"))
        })
        .unwrap_or(crate::config::home()?.join(".config/gh"));
    digest.update(dir.as_os_str().as_encoded_bytes());
    match std::fs::read(dir.join("hosts.yml")) {
        Ok(bytes) => digest.update(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    // Only this one-way fingerprint is persisted, never the token or config.
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn cached(store: &Store, host: &str) -> Result<Option<Account>> {
    store.account(host, &credential()?)
}

pub(super) fn resolve(github: &GitHub, store: &Store, host: &str) -> Result<Account> {
    let credential = credential()?;
    if let Some(account) = store.account(host, &credential)? {
        return Ok(account);
    }
    // Try both quota resources: a depleted REST quota can still leave GraphQL
    // available. gh's auth report can also retain a configured active login.
    let login = github
        .quota_json(&["api", "user", "--hostname", host])
        .ok()
        .and_then(|json| json["login"].as_str().map(str::to_owned))
        .or_else(|| {
            github
                .quota_json(&[
                    "api",
                    "graphql",
                    "--hostname",
                    host,
                    "-f",
                    "query={viewer{login}}",
                ])
                .ok()
                .and_then(|json| json["data"]["viewer"]["login"].as_str().map(str::to_owned))
        })
        .or_else(|| {
            let json = github
                .quota_json(&[
                    "auth",
                    "status",
                    "--active",
                    "--hostname",
                    host,
                    "--json",
                    "hosts",
                ])
                .ok()?;
            json["hosts"][host].as_array()?.iter().find_map(|account| {
                (account["active"] == Value::Bool(true))
                    .then(|| account["login"].as_str().map(str::to_owned))
                    .flatten()
            })
        })
        .filter(|login| !login.is_empty())
        .context("could not identify the authenticated GitHub user for Quota wait")?;
    let account = Account::new(host, &login);
    store.remember(account.clone(), credential)?;
    Ok(account)
}
