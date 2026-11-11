//! One checked text presentation for local and received Inbox rows.
use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::logs::command_record::{CommandRecord, Issue};
use crate::logs::reading::{CurrentStage, Departure, Group, Health, NeedsYouKind};

pub(super) struct Row {
    pub group: Group,
    pub landing: bool,
    meta: Metadata,
    command: Command,
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
    github_ci: Option<crate::github::InboxCi>,
    github_mergeable: Option<crate::github::InboxMergeable>,
    #[serde(flatten)]
    departure: Option<Departure>,
    dismissal: Option<crate::logs::dismiss::DismissalView>,
}

enum Command {
    Record(Box<Record>),
    History(History),
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
        if value["source"] == "command_record" {
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
        }
    }

    fn print(&self, state: &str, indent: &str, machine: Option<&str>) {
        match self {
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
    pub fn read(value: &Value) -> Result<Self> {
        let meta: Metadata = serde_json::from_value(value.clone())?;
        Ok(Self {
            group: meta.group,
            landing: meta.landing,
            command: Command::read(value)?,
            meta,
        })
    }

    pub fn order(&self) -> crate::logs::reading::RowOrder {
        self.group
            .row_order(self.meta.needs_you_kind, self.command.ended_or_started())
    }

    pub fn print(&self, tags: bool) {
        let meta = &self.meta;
        let state = self
            .group
            .state_word(meta.needs_you_kind, meta.stopping, self.command.state());
        self.command
            .print(state, "", tags.then_some(meta.machine.as_str()));
        if self.group == Group::ReadyForReview {
            println!("  {}", meta.github_ci.unwrap_or_default().label());
            if meta.github_mergeable == Some(crate::github::InboxMergeable::Conflicting) {
                println!("  Merge conflict");
            }
        }
        if let Some(departure) = &meta.departure {
            println!("  {}", departure.line());
        }
        if let Some(dismissal) = &meta.dismissal {
            println!("  {}", dismissal.line());
        }
        if meta.count > 1
            && let Command::Record(record) = &self.command
        {
            println!(
                "  {} Commands with this cause: {}",
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
