//! Read machine-wide Command records without entering a Launch directory.

mod history;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::Serialize;

use super::command_record::CommandRecord;
use super::effects::OnMachine;
use crate::config::UserConfig;

#[derive(Serialize)]
struct Status {
    version: u32,
    hostname: String,
    commands: Vec<Entry>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Entry {
    Record(Box<Command>),
    History(Box<history::Command>),
}

impl Entry {
    fn ordering(
        &self,
        all: bool,
    ) -> (
        bool,
        std::cmp::Reverse<Option<DateTime<chrono::FixedOffset>>>,
        &str,
    ) {
        let (state, started, repo) = match self {
            Self::Record(command) => (
                command.state,
                command.record.started_at.as_str(),
                command.record.repository.as_str(),
            ),
            Self::History(command) => (
                command.state,
                command.started_at.as_str(),
                command.repository.as_str(),
            ),
        };
        (
            !all && state != "live",
            std::cmp::Reverse(DateTime::parse_from_rfc3339(started).ok()),
            repo,
        )
    }
}

#[derive(Serialize)]
struct Command {
    #[serde(flatten)]
    record: CommandRecord,
    state: &'static str,
    stage: Option<CurrentStage>,
    source: &'static str,
    command_log: PathBuf,
}

#[derive(Serialize)]
struct CurrentStage {
    name: String,
    began_at: String,
    age_seconds: i64,
}

pub fn show(json: bool, all: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let now = Local::now();
    let mut commands = Vec::new();
    for owner in directories(&config.logs_dir)? {
        for repo in directories(&owner)? {
            let mut covered = HashSet::new();
            for folder in directories(&repo.join("commands"))? {
                for entry in entries(&folder)? {
                    if !entry.file_type()?.is_file()
                        || !entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(".command.json")
                    {
                        continue;
                    }
                    let path = entry.path();
                    let bytes = fs::read(&path)
                        .with_context(|| format!("could not read {}", path.display()))?;
                    let Ok(mut record) = serde_json::from_slice::<CommandRecord>(&bytes) else {
                        continue;
                    };
                    if record.version != 1 {
                        continue;
                    }
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    let command_log = path
                        .with_file_name(format!("{}.log", name.split('.').next().unwrap_or(&name)));
                    covered.insert(command_log.clone());
                    // Records written before completion times existed were last
                    // replaced when their outcome was recorded.
                    if record.outcome.is_some() && record.ended_at.is_none() {
                        record.ended_at = entry
                            .metadata()?
                            .modified()
                            .ok()
                            .map(|time| DateTime::<Local>::from(time).to_rfc3339());
                    }
                    if !all && record.outcome.is_some() && !recent(record.ended_at.as_deref(), now)
                    {
                        continue;
                    }
                    let state = if record.outcome.is_some() {
                        "ended"
                    } else if record.live(&mut OnMachine) {
                        "live"
                    } else {
                        "died"
                    };
                    let stage = record.current_stage().map(|stage| CurrentStage {
                        name: stage.name.clone(),
                        began_at: stage.began_at.clone(),
                        age_seconds: DateTime::parse_from_rfc3339(&stage.began_at)
                            .map(|began| now.signed_duration_since(began).num_seconds().max(0))
                            .unwrap_or(0),
                    });
                    commands.push(Entry::Record(Box::new(Command {
                        record,
                        state,
                        stage,
                        source: "command_record",
                        command_log,
                    })));
                }
            }
            commands.extend(
                history::read(&repo, &covered)?
                    .into_iter()
                    .filter(|command| all || recent(command.ended_at.as_deref(), now))
                    .map(|command| Entry::History(Box::new(command))),
            );
        }
    }
    commands.sort_by(|a, b| a.ordering(all).cmp(&b.ordering(all)));
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        commands,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        for entry in status.commands {
            let command = match entry {
                Entry::Record(command) => command,
                Entry::History(command) => {
                    command.print();
                    continue;
                }
            };
            let Command {
                record: facts,
                stage,
                state,
                ..
            } = *command;
            let stage = facts.outcome.clone().unwrap_or_else(|| {
                stage.map_or_else(
                    || "starting".into(),
                    |stage| {
                        if state == "died" {
                            format!("died at {}", stage.name)
                        } else {
                            format!("{} · {}", stage.name, age(stage.age_seconds))
                        }
                    },
                )
            });
            println!(
                "{}#{} {} · {} · {} · {} · {stage}",
                facts.repository,
                facts.issue.number,
                facts
                    .issue
                    .title
                    .as_deref()
                    .unwrap_or("(title unavailable)"),
                facts.harness,
                facts.model.as_deref().unwrap_or("default Model"),
                facts.effort.as_deref().unwrap_or("default Effort")
            );
        }
    }
    Ok(())
}

fn recent(ended: Option<&str>, now: DateTime<Local>) -> bool {
    ended
        .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
        .is_some_and(|ended| ended >= now - chrono::Duration::hours(24))
}

fn age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds} sec")
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} hr {} min", seconds / 3600, seconds % 3600 / 60)
    }
}

fn entries(path: &Path) -> Result<Vec<fs::DirEntry>> {
    match fs::read_dir(path) {
        Ok(entries) => entries.collect::<std::io::Result<_>>().map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("could not list {}", path.display())),
    }
}

fn directories(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in entries(path)? {
        if entry.file_type()?.is_dir() {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}
