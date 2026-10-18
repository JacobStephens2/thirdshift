//! Read machine-wide Command records without entering a Launch directory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::Serialize;

use super::command_record::{CommandRecord, CommandTree};
use super::effects::OnMachine;
use super::{Pass, activity};
use crate::config::UserConfig;
use crate::issue::IssueUrl;

#[derive(Serialize)]
struct Status {
    version: u32,
    hostname: String,
    commands: Vec<Command>,
    idle_passes: Vec<IdlePass>,
}

#[derive(Serialize)]
struct IdlePass {
    repository: String,
    kind: String,
    reason: String,
}

#[derive(Serialize)]
struct Command {
    #[serde(flatten)]
    record: CommandRecord,
    state: &'static str,
    stage: Option<CurrentStage>,
    children: Vec<Command>,
    dispatched: Vec<Command>,
}

#[derive(Serialize)]
struct CurrentStage {
    name: String,
    began_at: String,
    age_seconds: i64,
}

impl Command {
    fn new(record: CommandRecord, now: DateTime<Local>) -> Self {
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
        Self {
            record,
            state,
            stage,
            children: Vec::new(),
            dispatched: Vec::new(),
        }
    }

    fn print(&self, indent: &str) {
        let facts = &self.record;
        let stage = stage_text(self);
        let subject = match &facts.issue {
            Some(issue) => format!(
                "{}#{} {}",
                facts.repository,
                issue.number,
                issue.title.as_deref().unwrap_or("(title unavailable)")
            ),
            None => format!("{} on {}", facts.name, facts.repository),
        };
        println!(
            "{indent}{} {subject} · {} · {} · {} · {stage}",
            facts.kind,
            facts.harness,
            facts.model.as_deref().unwrap_or("default Model"),
            facts.effort.as_deref().unwrap_or("default Effort")
        );
        print_tickets(facts, &format!("{indent}  "));
        for dispatched in &self.dispatched {
            dispatched.print(&format!("{indent}  "));
        }
        for child in &self.children {
            child.print(&format!("{indent}  "));
        }
    }
}

pub fn show(json: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let now = Local::now();
    let mut records = BTreeMap::new();
    let mut idle_passes = Vec::new();
    for owner in directories(&config.logs_dir)? {
        for repo in directories(&owner)? {
            idle_passes.extend(read_idle_passes(&repo)?);
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
                    let Ok(tree) = serde_json::from_slice::<CommandTree>(&bytes) else {
                        continue;
                    };
                    if tree.command.version != 1 {
                        continue;
                    }
                    records.insert(path, tree);
                }
            }
        }
    }
    let roots: Vec<_> = records
        .iter()
        .filter(|(_, tree)| tree.command.parent_record.is_none() && tree.command.outcome.is_none())
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
    idle_passes.sort_by(|a, b| (&a.repository, &a.kind).cmp(&(&b.repository, &b.kind)));
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        commands,
        idle_passes,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        for command in &status.commands {
            command.print("");
        }
        for pass in status.idle_passes {
            println!("{} idle on {}: {}", pass.kind, pass.repository, pass.reason);
        }
    }
    Ok(())
}

fn tree(
    path: &Path,
    records: &mut BTreeMap<PathBuf, CommandTree>,
    now: &DateTime<Local>,
) -> Command {
    let tree = records.remove(path).expect("a discovered record exists");
    let mut command = Command::new(tree.command, *now);
    command.children = children(path, None, records, now);
    command.dispatched = tree
        .dispatched
        .into_iter()
        .map(|record| {
            let mut dispatched = Command::new(record, *now);
            let issue = dispatched
                .record
                .issue
                .as_ref()
                .map(|issue| issue.url.as_str());
            dispatched.children = children(path, issue, records, now);
            dispatched
        })
        .collect();
    command
}

fn children(
    path: &Path,
    dispatch: Option<&str>,
    records: &mut BTreeMap<PathBuf, CommandTree>,
    now: &DateTime<Local>,
) -> Vec<Command> {
    let paths: Vec<_> = records
        .iter()
        .filter(|(_, child)| {
            child.command.parent_record.as_deref() == Some(path)
                && child.command.parent_dispatch.as_deref() == dispatch
        })
        .map(|(path, _)| path.clone())
        .collect();
    let mut children: Vec<_> = paths
        .into_iter()
        .map(|path| tree(&path, records, now))
        .collect();
    children.sort_by_key(|child| child.record.issue.as_ref().map(|issue| issue.number));
    children
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

fn print_tickets(facts: &CommandRecord, indent: &str) {
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
        println!("{indent}Ticket #{} · {state}", ticket.issue.number);
    }
}

fn read_idle_passes(repo: &Path) -> Result<Vec<IdlePass>> {
    let path = repo.join(activity::FILE);
    let activity = match fs::read_to_string(&path) {
        Ok(activity) => activity,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let repository = format!(
        "{}/{}",
        repo.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        repo.file_name().unwrap().to_string_lossy()
    );
    let mut idle = Vec::new();
    for pass in [Pass::Architect, Pass::Pickup, Pass::Security] {
        let kind = activity::Kind::Pass(pass);
        if let Some(line) = activity::last_of(&activity, kind)
            && let Some(reason) = line.strip_prefix(&format!("{kind} skipped: "))
        {
            idle.push(IdlePass {
                repository: repository.clone(),
                kind: kind.to_string(),
                reason: reason.into(),
            });
        }
    }
    Ok(idle)
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
