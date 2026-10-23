//! Current-layout log inventory shared by the CLI and Console. IDs name exact
//! retained files, including older Commands with the same issue or start stamp.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use serde::Serialize;

use super::{Command, Entry, Observation, Snapshot, directories, entries};
use crate::config::UserConfig;
use crate::logs::owner;

#[derive(Clone, Copy, Serialize)]
pub(crate) enum Kind {
    #[serde(rename = "Command log")]
    Command,
    #[serde(rename = "Session trace")]
    Session,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Command => "Command log",
            Self::Session => "Session trace",
        })
    }
}

#[derive(Serialize)]
pub(crate) struct Log {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub kind: Kind,
    pub live: bool,
    pub command: String,
    pub owners: Vec<String>,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    pub selector: String,
}

#[derive(Default)]
struct Observed {
    live: bool,
    command: String,
    owners: Vec<String>,
    selector: String,
}

pub(crate) fn id(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn load(config: &UserConfig, selected_owner: Option<&str>) -> Result<Vec<Log>> {
    let root = &config.logs_dir;
    let snapshot = Snapshot::load(root, Local::now())?;
    // A child Run keeps its own Sessions, while sharing its Command's log.
    let mut observed = BTreeMap::new();
    fn collect(
        command: &Command,
        root: &Path,
        ancestors: &[String],
        observed: &mut BTreeMap<PathBuf, Observed>,
    ) -> Result<()> {
        let mut owners = ancestors.to_vec();
        owners.push(id(command.record_path.strip_prefix(root)?));
        let subject = command
            .record
            .issue
            .as_ref()
            .map(|issue| format!("{}#{}", command.record.repository, issue.number))
            .unwrap_or_else(|| command.record.name.clone());
        let observation = Observation::Record(command);
        let selector = observation.selector().context("Command has no selector")?;
        observed
            .entry(command.command_log.clone())
            .or_insert(Observed {
                live: command.state == "live",
                command: subject.clone(),
                owners: owners.clone(),
                selector: selector.clone(),
            });
        let current = command
            .record
            .session_log
            .as_deref()
            .map(crate::logs::session::path);
        for session in Observation::Record(command).sessions()? {
            let session = crate::logs::session::path(&session);
            let live = command.state == "live" && current.as_ref() == Some(&session);
            observed.insert(
                session,
                Observed {
                    live,
                    command: subject.clone(),
                    owners: owners.clone(),
                    selector: selector.clone(),
                },
            );
        }
        for child in command.dispatched.iter().chain(&command.children) {
            collect(child, root, &owners, observed)?;
        }
        Ok(())
    }
    for entry in &snapshot.commands {
        if let Entry::Record(command) = entry {
            collect(command, root, &[], &mut observed)?;
        }
    }
    let mut logs = Vec::new();
    for owner_dir in directories(root)? {
        for repo in directories(&owner_dir)? {
            let repository = format!(
                "{}/{}",
                owner_dir.file_name().unwrap_or_default().to_string_lossy(),
                repo.file_name().unwrap_or_default().to_string_lossy()
            );
            for folder in directories(&repo.join("commands"))? {
                for entry in entries(&folder)? {
                    let path = entry.path();
                    if !entry.file_type()?.is_file()
                        || !owner::inventory_session(&path)
                        || !matches!(
                            path.extension().and_then(|ext| ext.to_str()),
                            Some("log" | "jsonl")
                        )
                    {
                        continue;
                    }
                    let path = if path.extension().is_some_and(|ext| ext == "jsonl") {
                        crate::logs::session::path(&path)
                    } else {
                        path
                    };
                    let relative = path.strip_prefix(root)?;
                    let Observed {
                        live,
                        command,
                        mut owners,
                        selector,
                    } = observed.remove(&path).unwrap_or_default();
                    if let Some(decoded) = owner::decode(&path) {
                        owners.push(id(decoded.command_log(&path).strip_prefix(root)?));
                    }
                    logs.push(Log {
                        id: id(relative),
                        name: relative.to_string_lossy().into_owned(),
                        repository: repository.clone(),
                        kind: if path.extension().is_some_and(|ext| ext == "jsonl") {
                            Kind::Session
                        } else {
                            Kind::Command
                        },
                        live,
                        command,
                        owners,
                        selector: if selector.is_empty() {
                            path.to_string_lossy().into_owned()
                        } else {
                            selector
                        },
                        path,
                    });
                }
            }
        }
    }
    if let Some(selected) = selected_owner {
        logs.retain(|log| log.owners.iter().any(|owner| owner == selected));
    }
    logs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(logs)
}

pub(crate) fn find(config: &UserConfig, selected: &str) -> Result<Log> {
    load(config, None)?
        .into_iter()
        .find(|log| log.id == selected)
        .context("log is not in the current inventory")
}

pub(crate) fn show(json: bool, owner: Option<&str>) -> Result<()> {
    let logs = load(&UserConfig::load()?, owner)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"version": 1, "logs": logs}))?
        );
    } else {
        for log in logs {
            println!(
                "{} · {}{} · {} · {}",
                log.id,
                log.kind,
                if log.live { " · live" } else { "" },
                log.command,
                log.name
            );
        }
    }
    Ok(())
}

pub(crate) fn print_file(selected: &str, follow: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let log = find(&config, selected)?;
    let mut file = std::fs::File::open(&log.path)?;
    let mut stdout = std::io::stdout().lock();
    loop {
        let live = follow && find(&config, selected)?.live;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        stdout.write_all(&bytes)?;
        stdout.flush()?;
        if !live {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
