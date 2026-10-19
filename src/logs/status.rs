//! Format machine-wide Command observations.

pub mod inspect;

use anyhow::Result;
use chrono::Local;
use serde::Serialize;

use super::command_record::CommandRecord;
use super::reading::{Command, Entry, Group, IdlePass, InboxItem, Snapshot};
use crate::config::UserConfig;
use crate::issue::IssueUrl;

#[derive(Serialize)]
struct Status<'a> {
    version: u32,
    hostname: String,
    commands: Vec<InboxItem<'a>>,
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
        "{indent}{} · {} {subject} · {} · {} · {} · {stage}",
        command.state,
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
    let waiting = config.effective_waiting_line_interval();
    if waiting != config.waiting_line_interval {
        eprintln!(
            "thirdshift: warning: logs.waiting_line_minutes must be less than inbox.silent_minutes; using half the silent time ({} sec) for waiting lines",
            waiting.as_secs()
        );
    }
    let snapshot =
        Snapshot::load_with_silence(&config.logs_dir, Local::now(), config.silent_interval)?;
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        commands: snapshot.inbox(all, config.inbox_days),
        idle_passes: &snapshot.idle_passes,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        for group in Group::ORDER {
            let items: Vec<_> = status
                .commands
                .iter()
                .filter(|item| item.group == group)
                .collect();
            let references: Vec<_> = status
                .commands
                .iter()
                .filter(|item| {
                    group == Group::Landing && item.landing && item.group == Group::NeedsYou
                })
                .collect();
            if items.is_empty() && references.is_empty() {
                continue;
            }
            println!("{}:", group.title());
            for item in items {
                match item.command {
                    Entry::Record(command) => print_command(command, ""),
                    Entry::History(command) => print_history(command),
                }
                if item.count > 1 {
                    let cause = match item.command {
                        Entry::Record(command) => {
                            command.record.cause.as_deref().unwrap_or_default()
                        }
                        Entry::History(command) => command
                            .outcome
                            .as_deref()
                            .unwrap_or_default()
                            .trim_start_matches("failed: "),
                    };
                    println!("  {} Commands with this cause: {cause}", item.count);
                }
            }
            for item in references {
                if let Entry::Record(command) = item.command {
                    let facts = &command.record;
                    let name = facts.issue.as_ref().map_or_else(
                        || facts.name.clone(),
                        |issue| format!("{}#{}", facts.repository, issue.number),
                    );
                    println!(
                        "{name} · {} · see Needs you",
                        item.needs_you_kind.expect("Needs you has a kind").title()
                    );
                }
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
    let mut text = command.stage.as_ref().map_or_else(
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
    );
    if let Some(health) = command
        .health
        .as_ref()
        .and_then(super::reading::Health::text)
    {
        text.push_str(&format!(" · {health}"));
    }
    text
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
