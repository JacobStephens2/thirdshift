//! One checked text presentation for local and received Inbox rows.
use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::logs::command_record::{CommandRecord, Issue};
use crate::logs::github::PullRequestChecks;
use crate::logs::reading::{CurrentStage, Departure, Group, Health, NeedsYouKind};

use super::ready;

pub(super) struct Row {
    pub group: Group,
    pub landing: bool,
    meta: Metadata,
    command: Command,
    selector: String,
    pull_request_title: Option<String>,
    stale_github: bool,
}

#[derive(Deserialize)]
struct Metadata {
    #[serde(rename = "machine_name")]
    machine: String,
    group: Group,
    needs_you_kind: Option<NeedsYouKind>,
    landing: bool,
    stopping: bool,
    count: usize,
    #[serde(flatten)]
    github_checks: PullRequestChecks,
    #[serde(flatten)]
    departure: Option<Departure>,
    dismissal: Option<crate::logs::dismiss::DismissalView>,
}

enum Command {
    Record(Box<Record>),
    History(History),
    Reservation(Box<Reservation>),
}

#[derive(Deserialize)]
struct Reservation {
    repository: String,
    started_at: String,
    reservation: serde_json::Value,
    blocking: String,
    owner_description: String,
    release_command: String,
}

struct Record {
    facts: CommandRecord,
    view: View,
    children: Vec<Command>,
}

#[derive(Deserialize)]
struct View {
    account: Option<String>,
    state: String,
    stage: Option<CurrentStage>,
    health: Option<Health>,
    children: Vec<Value>,
    dispatched: Vec<Value>,
}

#[derive(Deserialize)]
struct History {
    account: Option<String>,
    repository: String,
    kind: String,
    issue: Option<Issue>,
    started_at: String,
    ended_at: Option<String>,
    outcome: Option<String>,
}

impl Command {
    fn read(value: &Value) -> Result<Self> {
        if value["source"] == "reservation" {
            Ok(Self::Reservation(Box::new(serde_json::from_value(
                value.clone(),
            )?)))
        } else if value["source"] == "command_record" {
            let view: View = serde_json::from_value(value.clone())?;
            let children = view
                .dispatched
                .iter()
                .chain(&view.children)
                .map(Self::read)
                .collect::<Result<_>>()?;
            Ok(Self::Record(Box::new(Record {
                facts: serde_json::from_value(value.clone())?,
                view,
                children,
            })))
        } else {
            Ok(Self::History(serde_json::from_value(value.clone())?))
        }
    }

    fn state(&self) -> &str {
        match self {
            Self::Record(record) => &record.view.state,
            Self::History(_) => "unknown",
            Self::Reservation(_) => "held",
        }
    }

    fn ended_or_started(&self) -> Option<&str> {
        match self {
            Self::Record(record) => record
                .facts
                .ended_at
                .as_deref()
                .or(Some(&record.facts.started_at)),
            Self::History(history) => history.ended_at.as_deref().or(Some(&history.started_at)),
            Self::Reservation(row) => Some(&row.started_at),
        }
    }

    fn print(&self, state: &str, indent: &str, machine: Option<&str>) {
        match self {
            Self::Reservation(row) => {
                println!(
                    "{indent}{}{} · {state} · {}",
                    machine
                        .map(|machine| format!("[{machine}] "))
                        .unwrap_or_default(),
                    row.repository,
                    row.reservation["reference"].as_str().unwrap_or_default()
                );
                let held = chrono::DateTime::parse_from_rfc3339(&row.started_at)
                    .map(|time| {
                        (chrono::Utc::now() - time.with_timezone(&chrono::Utc)).num_seconds()
                    })
                    .unwrap_or_default();
                println!(
                    "{indent}  {} · Held for {}",
                    row.owner_description,
                    super::age(held)
                );
                println!(
                    "{indent}  {}\n{indent}  {}",
                    row.blocking, row.release_command
                );
            }
            Self::Record(record) => {
                let stage = super::stage_details(
                    record.facts.outcome.as_deref(),
                    &record.view.state,
                    record.view.stage.as_ref(),
                    record.view.health.as_ref(),
                    &record.facts.waiting_for,
                );
                super::print_command_line(
                    &record.facts,
                    &stage,
                    record.view.account.as_deref(),
                    state,
                    indent,
                    machine,
                );
                super::print_tickets(&record.facts, &format!("{indent}  "));
                super::print_pause(&record.facts, indent);
                for child in &record.children {
                    child.print(child.state(), &format!("{indent}  "), machine);
                }
            }
            Self::History(history) => {
                let account = history
                    .account
                    .as_ref()
                    .map(|account| format!("[{account}] "))
                    .unwrap_or_default();
                let issue = history
                    .issue
                    .as_ref()
                    .map(|issue| format!("#{}", issue.number))
                    .unwrap_or_default();
                println!(
                    "{indent}{}{account}{}{issue} {} · {} → {} · {}",
                    machine
                        .map(|machine| format!("[{machine}] "))
                        .unwrap_or_default(),
                    history.repository,
                    crate::logs::display_name(&history.kind),
                    history.started_at,
                    history.ended_at.as_deref().unwrap_or("unknown end"),
                    history.outcome.as_deref().unwrap_or("unknown outcome")
                );
            }
        }
    }
}

impl Row {
    pub fn read(value: &Value, stale_github: bool) -> Result<Self> {
        let meta: Metadata = serde_json::from_value(value.clone())?;
        let pull_request = value["pull_request"].as_str().unwrap_or_default();
        Ok(Self {
            group: meta.group,
            landing: meta.landing,
            command: Command::read(value)?,
            selector: super::document_selector(value),
            pull_request_title: super::ready::title(value, pull_request).map(str::to_owned),
            stale_github,
            meta,
        })
    }

    fn ready_member(&self) -> ready::Member<'_> {
        let (repository, pull_request, account) = match &self.command {
            Command::Record(record) => (
                record.facts.repository.as_str(),
                record.facts.pull_request.as_deref().unwrap_or_default(),
                record.view.account.as_deref(),
            ),
            Command::History(history) => {
                (history.repository.as_str(), "", history.account.as_deref())
            }
            Command::Reservation(row) => (row.repository.as_str(), "", None),
        };
        ready::Member {
            repository,
            pull_request,
            order: self.order(),
            machine: &self.meta.machine,
            account,
            selector: &self.selector,
        }
    }

    pub fn order(&self) -> crate::logs::reading::RowOrder {
        self.group
            .row_order(self.meta.needs_you_kind, self.command.ended_or_started())
    }

    pub fn print(&self, tags: bool) {
        self.print_indented(tags, "");
    }

    fn print_indented(&self, tags: bool, indent: &str) {
        let meta = &self.meta;
        let state = self
            .group
            .state_word(meta.needs_you_kind, meta.stopping, self.command.state());
        self.command
            .print(state, indent, tags.then_some(meta.machine.as_str()));
        if self.group == Group::ReadyForReview {
            println!("{indent}  {}", meta.github_checks.ci().label());
            if meta.github_checks.has_merge_conflict() {
                println!("{indent}  Merge conflict");
            }
            if let Some(note) = &meta.github_checks.stale_ci {
                println!("{indent}  {note}");
            }
            if self.stale_github {
                println!("{indent}  Stale GitHub facts");
            }
        }
        if let Some(departure) = &meta.departure {
            println!("{indent}  {}", departure.line());
        }
        if let Some(dismissal) = &meta.dismissal {
            println!("{indent}  {}", dismissal.line());
        }
        if meta.count > 1
            && let Command::Record(record) = &self.command
        {
            println!(
                "{indent}  {} Commands with this cause: {}",
                meta.count,
                record.facts.cause.as_deref().unwrap_or_default()
            );
        }
    }

    pub fn print_reference(&self, tags: bool) {
        if let Command::Record(record) = &self.command {
            let name = record
                .facts
                .issue
                .as_ref()
                .map(|issue| format!("{}#{}", record.facts.repository, issue.number))
                .unwrap_or_else(|| record.facts.name.clone());
            let account = record
                .view
                .account
                .as_ref()
                .map(|account| format!("[{account}] "))
                .unwrap_or_default();
            println!(
                "{}{account}{name} · {} · see Needs you",
                if tags {
                    format!("[{}] ", self.meta.machine)
                } else {
                    String::new()
                },
                self.meta
                    .needs_you_kind
                    .map(|kind| kind.title())
                    .unwrap_or("Needs you")
            );
        }
    }
}

pub(super) fn print_ready(rows: Vec<&Row>, tags: bool) {
    for group in ready::group(rows, |row| row.ready_member()) {
        let Some(pr) = group.pull_request else {
            group.members[0].print(tags);
            continue;
        };
        let newest = group.members[0];
        // The title follows the validated URL on this line. Document text can
        // carry a carriage return or escape, which a terminal would use to
        // erase that URL, so render it as visible text first.
        let title = newest
            .pull_request_title
            .as_deref()
            .unwrap_or("(title unavailable)");
        println!(
            "{} · {} · {} · {} Command{}",
            pr.label,
            pr.url,
            crate::cronjobs::report_line(title),
            group.members.len(),
            if group.members.len() == 1 { "" } else { "s" },
        );
        for member in group.members {
            member.print_indented(true, "  ");
            if let Command::Record(record) = &member.command {
                println!(
                    "    Command: {}",
                    crate::cronjobs::report_line(&member.selector)
                );
                println!(
                    "    Started {} · Ended {}",
                    crate::cronjobs::report_line(&record.facts.started_at),
                    crate::cronjobs::report_line(
                        record.facts.ended_at.as_deref().unwrap_or("unknown")
                    )
                );
            }
        }
    }
}
