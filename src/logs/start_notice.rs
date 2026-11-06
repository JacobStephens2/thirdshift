//! Private, short-lived start handoff to the Console that spawned this process.
//! Unlike output or Activity history, it also works for quiet, repeated skips.
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const ENV: &str = "THIRDSHIFT_START_NOTICE";

#[derive(Deserialize, Serialize)]
struct Notice {
    process_id: u32,
    response: Value,
}

pub(crate) fn skipped(reason: impl std::fmt::Display) {
    send(json!({"version":1,"outcome":"skipped","reason":reason.to_string()}));
}

/// Skill runs have no Command record in the CLI. Record an attributed API
/// start after preflight, then acknowledge the detached process instead.
pub(crate) fn skill_started(config: &crate::config::UserConfig, name: &str) -> Result<()> {
    if std::env::var_os(ENV).is_some() {
        let actor =
            super::Actor::starting().ok_or_else(|| anyhow::anyhow!("missing start requester"))?;
        let path = super::root_under(&config.logs_dir, &crate::launch::repo()?)
            .join(super::activity::FILE);
        writeln!(
            super::activity::open(&path)?,
            "{} Skill {name} started {actor}",
            chrono::Local::now().format(super::activity::TIME_FORMAT)
        )?;
        send(json!({"version":1,"outcome":"started","process_id":std::process::id()}));
    }
    Ok(())
}

fn send(response: Value) {
    let Some(path) = std::env::var_os(ENV) else {
        return;
    };
    // The Console owns the file. Never recreate it after that Console exits.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
    {
        let notice = Notice {
            process_id: std::process::id(),
            response,
        };
        let _ = serde_json::to_writer(&mut file, &notice);
        let _ = file.flush();
    }
}

pub(crate) fn read(path: &Path, process_id: u32) -> Result<Option<Value>> {
    let bytes = std::fs::read(path)?;
    // A running child may still be writing. Only its own notice can answer.
    Ok(serde_json::from_slice::<Notice>(&bytes)
        .ok()
        .filter(|notice| notice.process_id == process_id)
        .map(|notice| notice.response))
}
