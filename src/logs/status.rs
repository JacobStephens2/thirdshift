//! Format machine-wide Command observations.

mod inbox;
pub mod inspect;
pub(crate) mod time;
mod watching;
pub(crate) use inbox::{Archive, ArchivePage, archive};
pub(crate) use inbox::{Inbox, Row, inbox, issue_url, subject};
pub(crate) use watching::{Process, watching};

use anyhow::Result;
use chrono::Local;
use serde::Serialize;

use super::command_record::CommandRecord;
use super::pass_starts::PassStart;
use super::reading::{Command, Group, IdlePass, InboxItem, Observation, Snapshot};
use crate::config::UserConfig;
use crate::issue::IssueUrl;

#[derive(Serialize)]
struct Status<'a> {
    version: u32,
    hostname: String,
    active_runs: usize,
    commands: Vec<InboxItem<'a>>,
    idle_passes: &'a [IdlePass],
    pass_starts: &'a [PassStart],
    /// How old the Inbox's kept GitHub answer is, in seconds. `None` where
    /// no answer is kept: with no ready pull requests to read, or before
    /// the first read.
    github_answer_age_seconds: Option<u64>,
    github_last_answered_at: Option<String>,
    github_failure: Option<&'a str>,
}

fn print_command(command: &Command, state_word: &str, indent: &str) {
    let facts = &command.record;
    let stage = stage_text(command);
    let mut subject = if let Some(name) = facts.process_label() {
        match &facts.issue {
            Some(issue) => format!(
                "{name} {}#{} {}",
                facts.repository,
                issue.number,
                issue.title.as_deref().unwrap_or("(title unavailable)")
            ),
            None => format!("{name} on {}", facts.repository),
        }
    } else {
        match &facts.issue {
            Some(issue) => format!(
                "{}#{} {}",
                facts.repository,
                issue.number,
                issue.title.as_deref().unwrap_or("(title unavailable)")
            ),
            None => format!("{} on {}", facts.name, facts.repository),
        }
    };
    if let Some(pull_request) = &facts.pull_request {
        subject.push_str(&format!(" · {pull_request}"));
    }
    if let Some(account) = &command.account {
        subject.push_str(&format!(" · [{account}]"));
    }
    println!(
        "{indent}{} {subject} · {} · {} · {} · {} · {stage}",
        super::display_name(&facts.kind),
        state_word,
        facts.harness,
        facts.model.as_deref().unwrap_or("default Model"),
        facts.effort.as_deref().unwrap_or("default Effort")
    );
    print_tickets(facts, &format!("{indent}  "));
    for dispatched in &command.dispatched {
        print_command(dispatched, dispatched.state, &format!("{indent}  "));
    }
    for child in &command.children {
        print_command(child, child.state, &format!("{indent}  "));
    }
}

pub(super) fn print_history(command: &super::reading::HistoricalCommand) {
    if let Some(account) = &command.account {
        print!("[{account}] ");
    }
    let issue = command
        .issue
        .as_ref()
        .map(|issue| format!("#{}", issue.number))
        .unwrap_or_default();
    println!(
        "{}{issue} {} · {} → {} · {}",
        command.repository,
        super::display_name(&command.kind),
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
    let snapshot = Snapshot::for_user(&config, Local::now())?;
    for account in &snapshot.unavailable_accounts {
        eprintln!("thirdshift: account {account}: logs unavailable or unreadable");
    }
    let kept = super::github::refresh(&snapshot, &config, super::github::FRESH_FOR)?;
    let status = Status {
        version: 1,
        hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
        active_runs: snapshot.active_runs(),
        commands: snapshot
            .evaluate_inbox(config.inbox_days, &kept)
            .into_rows(all),
        idle_passes: &snapshot.idle_passes,
        pass_starts: &snapshot.pass_starts,
        github_answer_age_seconds: kept.age.map(|age| age.as_secs()),
        github_last_answered_at: kept.last_answered_at(),
        github_failure: kept.failure.as_deref(),
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", status.hostname);
        println!("{}", active_runs_text(status.active_runs));
        if let Some(warning) = kept.warning() {
            println!("Warning: {warning}");
        }
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
                    Observation::Record(command) => print_command(command, item.state_word(), ""),
                    Observation::History(command) => print_history(command),
                }
                if item.group == Group::ReadyForReview {
                    println!("  {}", item.github_ci.unwrap_or_default().label());
                }
                if let Some(line) = item.left_line() {
                    println!("  {line}");
                }
                if let Some(dismissal) = &item.dismissal {
                    println!("  {}", dismissal.line());
                }
                if item.count > 1 {
                    let cause = item.cause().unwrap_or_default();
                    println!("  {} Commands with this cause: {cause}", item.count);
                }
            }
            for item in references {
                if let Observation::Record(command) = item.command {
                    let facts = &command.record;
                    let name = facts.issue.as_ref().map_or_else(
                        || facts.name.clone(),
                        |issue| format!("{}#{}", facts.repository, issue.number),
                    );
                    if let Some(account) = &command.account {
                        print!("[{account}] ");
                    }
                    println!(
                        "{name} · {} · see Needs you",
                        item.needs_you_kind.expect("Needs you has a kind").title()
                    );
                }
            }
        }
        for pass in status.idle_passes {
            println!("{}", pass.line());
        }
        for pass in status.pass_starts {
            println!("{}", pass.line());
        }
    }
    Ok(())
}

/// The Inbox's total of outer Command trees with live work, in every group.
pub(crate) fn active_runs_text(count: usize) -> String {
    format!("{count} active run{}", if count == 1 { "" } else { "s" })
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
                format!("died at {}", super::display_name(&stage.name))
            } else {
                format!(
                    "{} · {}",
                    super::display_name(&stage.name),
                    age(stage.age_seconds)
                )
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
        println!("{indent}{}", ticket_text(ticket, &facts.repository));
    }
}

fn ticket_text(ticket: &super::command_record::TicketRecord, repository: &str) -> String {
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
                    if IssueUrl::parse(&issue.url)
                        .is_ok_and(|blocker| blocker.repo_slug().eq_ignore_ascii_case(repository))
                    {
                        format!("#{}", issue.number)
                    } else {
                        issue.url.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    format!("Ticket #{} · {state}", ticket.issue.number)
}

pub(super) fn age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds} sec")
    } else if seconds < 3600 {
        format!("{} min", seconds / 60)
    } else {
        format!("{} hr {} min", seconds / 3600, seconds % 3600 / 60)
    }
}
