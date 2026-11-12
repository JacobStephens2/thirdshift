//! Format machine-wide Command observations.

pub(crate) mod fleet;
mod inbox;
pub mod inspect;
mod text;
pub(crate) mod time;
mod watching;
pub(crate) use inbox::validate_remote;
pub(crate) use inbox::{Archive, ArchivePage};
pub(crate) use inbox::{Inbox, Row, issue_url, subject, subject_link_matches};
pub(crate) use watching::{Process, watching};

use anyhow::Result;
use chrono::Local;
use serde::Serialize;

use super::command_record::CommandRecord;
use super::pass_starts::PassStart;
use super::reading::{Command, Group, IdlePass, InboxItem, Snapshot};
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
    github_quota_wait: Option<String>,
    /// How old the Inbox's kept GitHub answer is, in seconds. `None` where
    /// no answer is kept: with no ready pull requests to read, or before
    /// the first read.
    github_answer_age_seconds: Option<u64>,
    github_last_answered_at: Option<String>,
    github_failure: Option<&'a str>,
}

/// One captured read, rendered by the CLI, Console page or Console API.
pub(crate) struct Read {
    snapshot: Snapshot,
    kept: super::github::Kept,
    now: chrono::DateTime<Local>,
}

impl Read {
    pub fn capture(config: &UserConfig, refresh: bool) -> Result<Self> {
        let now = Local::now();
        let snapshot = Snapshot::for_user(config, now)?;
        let kept = if refresh {
            super::github::refresh(&snapshot, config, super::github::FRESH_FOR)?
        } else {
            super::github::load(&config.logs_dir)
        };
        Ok(Self {
            snapshot,
            kept,
            now,
        })
    }

    fn status(&self, config: &UserConfig, all: bool) -> Status<'_> {
        Status {
            version: fleet::VERSION,
            hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
            active_runs: self.snapshot.active_runs(),
            commands: self
                .snapshot
                .evaluate_inbox(config.inbox_days, &self.kept)
                .into_rows(all),
            idle_passes: &self.snapshot.idle_passes,
            pass_starts: &self.snapshot.pass_starts,
            github_quota_wait: self.kept.quota_wait(),
            github_answer_age_seconds: self.kept.age.map(|age| age.as_secs()),
            github_last_answered_at: self.kept.last_answered_at(),
            github_failure: self.kept.failure.as_deref(),
        }
    }

    pub fn json(&self, config: &UserConfig, all: bool) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self.document(config, all)?)?)
    }

    pub fn document(&self, config: &UserConfig, all: bool) -> Result<fleet::Document> {
        fleet::local(
            serde_json::to_value(self.status(config, all))?,
            self.now.into(),
        )
    }

    pub fn inbox(&self, config: &UserConfig) -> Result<Inbox> {
        inbox::from_read(config, &self.snapshot, &self.kept, self.now)
    }

    pub fn archive(
        &self,
        config: &UserConfig,
        page: ArchivePage,
        outcome: &str,
        repository: &str,
    ) -> Result<Archive> {
        inbox::archive_from_read(config, page, outcome, repository, &self.snapshot, self.now)
    }

    pub fn times(&self, hours: f64, by_repository: bool) -> Result<time::Views> {
        time::from_read(&self.snapshot, self.now, hours, by_repository)
    }
}

fn print_command_line(
    facts: &CommandRecord,
    stage: &str,
    account: Option<&str>,
    state_word: &str,
    indent: &str,
    machine: Option<&str>,
) {
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
    if let Some(account) = account {
        subject.push_str(&format!(" · [{account}]"));
    }
    let tag = machine
        .map(|machine| format!("[{machine}] "))
        .unwrap_or_default();
    println!(
        "{indent}{tag}{} {subject} · {} · {} · {} · {} · {stage}",
        super::display_name(&facts.kind),
        state_word,
        facts.harness,
        facts.model.as_deref().unwrap_or("default Model"),
        facts.effort.as_deref().unwrap_or("default Effort")
    );
}

pub(super) fn print_pause(facts: &super::command_record::CommandRecord, indent: &str) {
    if facts.outcome.as_deref() == Some("paused") {
        if let Some(cause) = &facts.cause {
            println!("{indent}  {cause}");
        }
        if let Some(command) = &facts.resume_command {
            println!("{indent}  Resume: {command}");
        }
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

pub fn show(json: bool, all: bool, machine: Option<&str>, local_only: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let waiting = config.effective_waiting_line_interval();
    if waiting != config.waiting_line_interval {
        eprintln!(
            "thirdshift: warning: logs.waiting_line_minutes must be less than inbox.silent_minutes; using half the silent time ({} sec) for waiting lines",
            waiting.as_secs()
        );
    }
    let read = Read::capture(&config, true)?;
    for account in &read.snapshot.unavailable_accounts {
        eprintln!("thirdshift: account {account}: logs unavailable or unreadable");
    }
    let status = read.status(&config, all);
    let local = fleet::local(serde_json::to_value(&status)?, read.now.into())?;
    let document = if local_only {
        local
    } else {
        fleet::fetch_and_merge(&config, local, machine, all)?
    };
    let include_local = machine.is_none_or(|machine| machine == status.hostname);
    let tags = document.machines.len() > 1 || !include_local;
    if json {
        println!("{}", serde_json::to_string_pretty(&document)?);
    } else {
        println!("{}", status.hostname);
        println!("{}", active_runs_text(document.active_runs));
        if tags {
            for machine in &document.machines {
                let reachability = serde_json::to_value(&machine.reachability)?;
                println!(
                    "{}: {} · {}{}",
                    machine.name,
                    active_runs_text(machine.active_runs),
                    reachability.as_str().unwrap(),
                    machine
                        .error
                        .as_ref()
                        .map(|error| format!(" · {error}"))
                        .unwrap_or_default()
                );
            }
        }
        if include_local && let Some(warning) = read.kept.warning() {
            println!("Warning: {warning}");
        }
        let rows: Vec<_> = document
            .commands
            .iter()
            .map(text::Row::read)
            .collect::<Result<_>>()?;
        for group in Group::ORDER {
            let items: Vec<_> = rows.iter().filter(|row| row.group == group).collect();
            let references: Vec<_> = rows
                .iter()
                .filter(|row| {
                    group == Group::Landing && row.landing && row.group == Group::NeedsYou
                })
                .collect();
            if items.is_empty() && references.is_empty() {
                continue;
            }
            println!("{}:", group.title());
            for item in items {
                item.print(tags);
            }
            for item in references {
                item.print_reference(tags);
            }
        }
        for pass in status.idle_passes.iter().filter(|_| include_local) {
            println!("{}", pass.line());
        }
        for pass in status.pass_starts.iter().filter(|_| include_local) {
            println!("{}", pass.line());
        }
        if include_local && let Some(wait) = status.github_quota_wait {
            println!("{wait}");
        }
    }
    Ok(())
}

/// The Inbox's total of outer Command trees with live work, in every group.
pub(crate) fn active_runs_text(count: usize) -> String {
    format!("{count} active run{}", if count == 1 { "" } else { "s" })
}

pub(super) fn stage_text(command: &Command) -> String {
    stage_details(
        command.record.outcome.as_deref(),
        command.state,
        command.stage.as_ref(),
        command.health.as_ref(),
        &command.record.waiting_for,
    )
}

fn stage_details(
    outcome: Option<&str>,
    state: &str,
    stage: Option<&super::reading::CurrentStage>,
    health: Option<&super::reading::Health>,
    waiting_for: &[String],
) -> String {
    if let Some(outcome) = outcome {
        return outcome.to_owned();
    }
    let mut text = stage.map_or_else(
        || {
            if state == "died" {
                "died before first stage".into()
            } else {
                "starting".into()
            }
        },
        |stage| {
            if state == "died" {
                format!("died at {}", super::display_name(&stage.name))
            } else {
                format!(
                    "{} · {}",
                    super::display_name(&stage.name),
                    stage_time(Some(stage.age_seconds as f64), stage.suspended_seconds)
                )
            }
        },
    );
    if state == "live" && !waiting_for.is_empty() {
        text.push_str(&format!(" · {}", waiting_for.join(", ")));
    }
    if let Some(health) = health.and_then(super::reading::Health::text) {
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

pub(super) fn stage_time(seconds: Option<f64>, suspended_seconds: u64) -> String {
    let mut time = seconds
        .map(|seconds| age(seconds as i64))
        .unwrap_or_else(|| "time unknown".into());
    if suspended_seconds > 0 {
        time.push_str(&format!(" · {} suspended", age(suspended_seconds as i64)));
    }
    time
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
