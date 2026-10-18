//! Read machine-wide Command records without entering a Launch directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::Serialize;

use super::command_record::CommandRecord;
use super::effects::OnMachine;
use crate::config::UserConfig;
use crate::issue::IssueUrl;

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
    children: Vec<Command>,
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
    let mut records = BTreeMap::new();
    for owner in directories(&config.logs_dir)? {
        for repo in directories(&owner)? {
            for folder in directories(&repo.join("commands"))? {
                for entry in entries(&folder)? {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    if !name.ends_with(".command.json") && !name.ends_with(".child.json") {
                        continue;
                    }
                    let path = entry.path();
                    let bytes = fs::read(&path)
                        .with_context(|| format!("could not read {}", path.display()))?;
                    let Ok(record) = serde_json::from_slice::<CommandRecord>(&bytes) else {
                        continue;
                    };
                    if record.version != 1 {
                        continue;
                    }
                    records.insert(path, record);
                }
            }
        }
    }
    let roots: Vec<_> = records
        .iter()
        .filter(|(_, record)| record.parent_record.is_none() && record.outcome.is_none())
        .map(|(path, _)| path.clone())
        .collect();
    let mut commands: Vec<_> = roots
        .into_iter()
        .map(|path| tree(&path, &mut records, &now))
        .collect();
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
            print_command(&command, 0);
        }
    }
    Ok(())
}

fn tree(
    path: &Path,
    records: &mut BTreeMap<PathBuf, CommandRecord>,
    now: &DateTime<Local>,
) -> Command {
    let record = records.remove(path).expect("a discovered record exists");
    let paths: Vec<_> = records
        .iter()
        .filter(|(_, child)| child.parent_record.as_deref() == Some(path))
        .map(|(path, _)| path.clone())
        .collect();
    let mut children: Vec<_> = paths
        .into_iter()
        .map(|path| tree(&path, records, now))
        .collect();
    children.sort_by_key(|child| child.record.issue.number);
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
    Command {
        record,
        state,
        stage,
        children,
    }
}

fn stage_text(command: &Command) -> String {
    if let Some(outcome) = &command.record.outcome {
        return outcome.clone();
    }
    command.stage.as_ref().map_or_else(
        || {
            if command.state == "died" {
                "died before first stage".into()
            } else {
                "starting".into()
            }
        },
        |stage| {
            if command.state == "died" {
                format!("died at {}", stage.name)
            } else {
                format!("{} · {}", stage.name, age(stage.age_seconds))
            }
        },
    )
}

fn print_command(command: &Command, depth: usize) {
    let facts = &command.record;
    println!(
        "{}{} {}#{} {} · {} · {} · {} · {}",
        "  ".repeat(depth),
        facts.kind,
        facts.repository,
        facts.issue.number,
        facts
            .issue
            .title
            .as_deref()
            .unwrap_or("(title unavailable)"),
        facts.harness,
        facts.model.as_deref().unwrap_or("default Model"),
        facts.effort.as_deref().unwrap_or("default Effort"),
        stage_text(command)
    );
    for ticket in &facts.tickets {
        use super::command_record::TicketState;
        let state = match ticket.state {
            TicketState::Running => "running".into(),
            TicketState::Merged => "merged".into(),
            TicketState::Failed => "failed".into(),
            TicketState::NotStarted => "not started".into(),
            TicketState::Waiting => format!(
                "waiting on {}",
                ticket
                    .blockers
                    .iter()
                    .map(|issue| {
                        if IssueUrl::parse(&issue.url).is_ok_and(|blocker| {
                            blocker.repo_slug().eq_ignore_ascii_case(&facts.repository)
                        }) {
                            format!("#{}", issue.number)
                        } else {
                            issue.url.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        println!(
            "{}Ticket #{} · {state}",
            "  ".repeat(depth + 1),
            ticket.issue.number
        );
    }
    for child in &command.children {
        print_command(child, depth + 1);
    }
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
