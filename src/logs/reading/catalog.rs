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

#[derive(Serialize)]
pub(crate) struct Log {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub kind: &'static str,
    pub live: bool,
    pub command: String,
    pub owners: Vec<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

pub(crate) fn id(path: &Path) -> String {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn load(config: &UserConfig) -> Result<Vec<Log>> {
    let root = &config.logs_dir;
    let snapshot = Snapshot::load(root, Local::now())?;
    // A child Run keeps its own Sessions, while sharing its Command's log.
    let mut observed = BTreeMap::new();
    fn collect(
        command: &Command,
        root: &Path,
        ancestors: &[String],
        observed: &mut BTreeMap<PathBuf, (bool, String, Vec<String>)>,
    ) -> Result<()> {
        let mut owners = ancestors.to_vec();
        owners.push(id(command.record_path.strip_prefix(root)?));
        let subject = command
            .record
            .issue
            .as_ref()
            .map(|issue| format!("{}#{}", command.record.repository, issue.number))
            .unwrap_or_else(|| command.record.name.clone());
        observed.entry(command.command_log.clone()).or_insert((
            command.state == "live",
            subject.clone(),
            owners.clone(),
        ));
        for session in Observation::Record(command).sessions()? {
            let live =
                command.state == "live" && command.record.session_log.as_ref() == Some(&session);
            observed.insert(session, (live, subject.clone(), owners.clone()));
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
                        || !matches!(
                            path.extension().and_then(|ext| ext.to_str()),
                            Some("log" | "jsonl")
                        )
                    {
                        continue;
                    }
                    let relative = path.strip_prefix(root)?;
                    let (live, command, mut owners) = observed.remove(&path).unwrap_or_default();
                    if let Some(decoded) = owner::decode(&path) {
                        owners.push(id(decoded.command_log(&path).strip_prefix(root)?));
                    }
                    logs.push(Log {
                        id: id(relative),
                        name: relative.to_string_lossy().into_owned(),
                        repository: repository.clone(),
                        kind: if path.extension().is_some_and(|ext| ext == "jsonl") {
                            "Session log"
                        } else {
                            "Command log"
                        },
                        live,
                        command,
                        owners,
                        path,
                    });
                }
            }
        }
    }
    logs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(logs)
}

pub(crate) fn find(config: &UserConfig, selected: &str) -> Result<Log> {
    load(config)?
        .into_iter()
        .find(|log| log.id == selected)
        .context("log is not in the current inventory")
}

pub(crate) fn show(json: bool) -> Result<()> {
    let logs = load(&UserConfig::load()?)?;
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
