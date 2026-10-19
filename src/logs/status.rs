//! Format machine-wide Command observations.

pub mod inspect;

use anyhow::Result;
use chrono::Local;
use serde::Serialize;

use super::command_record::CommandRecord;
use super::reading::{Command, Entry, IdlePass, Snapshot};
use crate::config::UserConfig;
use crate::issue::IssueUrl;

#[derive(Serialize)]
struct Status<'a> {
    version: u32,
    hostname: String,
    commands: Vec<&'a Entry>,
    idle_passes: &'a [IdlePass],
}

fn print_command(command: &Command, indent: &str) {
    let facts = &command.record;
    let stage = stage_text(command);
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
    for dispatched in &command.dispatched {
        print_command(dispatched, &format!("{indent}  "));
    }
    for child in &command.children {
        print_command(child, &format!("{indent}  "));
    }
}

pub(super) fn print_history(command: &super::reading::HistoricalCommand) {
    let issue = command
        .issue
        .as_ref()
        .map(|issue| format!("#{}", issue.number))
        .unwrap_or_default();
    println!(
        "{}{issue} {} · {} → {} · {}",
        command.repository,
        command.kind,
        command.started_at,
        command.ended_at.as_deref().unwrap_or("unknown end"),
        command.outcome.as_deref().unwrap_or("unknown outcome")
    );
}

pub fn show(json: bool, all: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load(&config.logs_dir, Local::now())?;
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        commands: snapshot.list(all),
        idle_passes: &snapshot.idle_passes,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        for entry in status.commands {
            match entry {
                Entry::Record(command) => print_command(command, ""),
                Entry::History(command) => print_history(command),
            }
        }
        for pass in status.idle_passes {
            println!("{} idle on {}: {}", pass.kind, pass.repository, pass.reason);
        }
    }
    Ok(())
}

pub(super) fn stage_text(command: &Command) -> String {
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

pub(super) fn print_tickets(facts: &CommandRecord, indent: &str) {
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

fn age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds} sec")
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} hr {} min", seconds / 3600, seconds % 3600 / 60)
    }
}
