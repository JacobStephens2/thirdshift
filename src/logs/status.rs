//! Read machine-wide Command records without entering a Launch directory.

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
    commands: Vec<Command>,
}

#[derive(Serialize)]
struct Command {
    #[serde(flatten)]
    record: CommandRecord,
    state: &'static str,
    stage: Option<CurrentStage>,
}

#[derive(Serialize)]
struct CurrentStage {
    name: String,
    began_at: String,
    age_seconds: i64,
}

pub fn show(json: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let now = Local::now();
    let mut commands = Vec::new();
    for owner in directories(&config.logs_dir)? {
        for repo in directories(&owner)? {
            for folder in directories(&repo.join("commands"))? {
                for entry in entries(&folder)? {
                    if !entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".command.json")
                    {
                        continue;
                    }
                    let path = entry.path();
                    let bytes = fs::read(&path)
                        .with_context(|| format!("could not read {}", path.display()))?;
                    let Ok(record) = serde_json::from_slice::<CommandRecord>(&bytes) else {
                        continue;
                    };
                    if record.version != 1 || record.outcome.is_some() {
                        continue;
                    }
                    let state = if record.live(&mut OnMachine) {
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
                    commands.push(Command {
                        record,
                        state,
                        stage,
                    });
                }
            }
        }
    }
    commands.sort_by(|a, b| {
        (
            a.state != "live",
            &a.record.repository,
            &a.record.started_at,
        )
            .cmp(&(
                b.state != "live",
                &b.record.repository,
                &b.record.started_at,
            ))
    });
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        commands,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        for command in status.commands {
            let facts = command.record;
            let stage = command.stage.map_or_else(
                || "starting".into(),
                |stage| {
                    if command.state == "died" {
                        format!("died at {}", stage.name)
                    } else {
                        format!("{} · {}", stage.name, age(stage.age_seconds))
                    }
                },
            );
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
