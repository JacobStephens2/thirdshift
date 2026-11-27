//! Admit the owning machine's Inbox facts while retaining its version-2 JSON.
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::logs::command_record::{CommandRecord, Issue, TicketRecord};
use crate::logs::dismiss::DismissalView;
use crate::logs::github::PullRequestChecks;
use crate::logs::reading::{CurrentStage, Departure, Group, Health, NeedsYouKind, RowOrder};

use super::fleet::Machine;

pub(super) const VERSION: u32 = 2;
const CHILD_FIELDS: [&str; 3] = ["dispatched", "children", "collapsed"];

pub(crate) type WireDocument = Envelope<Value>;
pub(super) type Document = Envelope<Row>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Envelope<T> {
    version: u32,
    pub hostname: String,
    pub active_runs: usize,
    pub(super) commands: Vec<T>,
    #[serde(default)]
    pub machines: Vec<Machine>,
    #[serde(flatten)]
    pub(super) other: Map<String, Value>,
}

#[derive(Clone, Serialize)]
#[serde(transparent)]
pub(crate) struct ConsoleDocument(Document);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AdmissionError {
    Tree,
    VersionOrRows,
    Presentation,
}

impl AdmissionError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Tree => "invalid Inbox Command tree",
            Self::VersionOrRows => "unsupported Inbox version or rows",
            Self::Presentation => "incompatible Inbox presentation",
        }
    }
}

impl WireDocument {
    pub(super) fn tag(&mut self, name: Option<&str>) -> Result<()> {
        let name = name.unwrap_or(&self.hostname);
        for value in &mut self.commands {
            tag(value, &self.hostname, name)?;
        }
        Ok(())
    }

    pub(super) fn admit_cli(mut self, name: &str) -> Result<Document, AdmissionError> {
        // Tree errors take precedence over version and row shape errors.
        self.tag(Some(name)).map_err(|_| AdmissionError::Tree)?;
        if self.version != VERSION {
            return Err(AdmissionError::VersionOrRows);
        }
        self.check_cli().map_err(|_| AdmissionError::VersionOrRows)
    }

    pub(super) fn check_cli(self) -> Result<Document> {
        let stale_github = super::stale_github_answer(
            self.other
                .get("github_answer_age_seconds")
                .and_then(Value::as_u64),
            self.other
                .get("github_failure")
                .is_some_and(Value::is_string),
        );
        let commands = self
            .commands
            .into_iter()
            .map(|value| Row::read(value, stale_github))
            .collect::<Result<_>>()?;
        Ok(Envelope {
            version: self.version,
            hostname: self.hostname,
            active_runs: self.active_runs,
            commands,
            machines: self.machines,
            other: self.other,
        })
    }

    pub(crate) fn admit_console(self, name: &str) -> Result<ConsoleDocument, AdmissionError> {
        let mut document = self.admit_cli(name)?;
        for row in &mut document.commands {
            row.command
                .check_console()
                .map_err(|_| AdmissionError::Presentation)?;
        }
        Ok(ConsoleDocument(document))
    }
}

impl ConsoleDocument {
    pub fn active_runs(&self) -> usize {
        self.0.active_runs
    }

    pub(super) fn rows(&self) -> &[Row] {
        &self.0.commands
    }

    pub fn omit_command(&mut self, selector: &str) {
        self.0.commands.retain_mut(|row| row.command.keep(selector));
    }
}

fn tag(row: &mut Value, hostname: &str, name: &str) -> Result<()> {
    let fields = row.as_object_mut().context("invalid Command")?;
    fields.insert("machine".into(), hostname.into());
    fields.insert("machine_name".into(), name.into());
    for field in CHILD_FIELDS {
        if let Some(nested) = fields.get_mut(field) {
            for child in nested.as_array_mut().context("invalid Command tree")? {
                tag(child, hostname, name)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(transparent)]
pub(super) struct Row {
    pub command: Command,
    #[serde(skip)]
    pub metadata: Arc<Metadata>,
    #[serde(skip)]
    pub stale_github: bool,
    #[serde(skip)]
    order: RowOrder,
}

impl Row {
    fn read(value: Value, stale_github: bool) -> Result<Self> {
        let metadata = Arc::new(Metadata::deserialize(&value)?);
        let command = Command::read(value, true)?;
        let order = metadata
            .group
            .row_order(metadata.needs_you_kind, command.cli().occurrence());
        Ok(Self {
            command,
            metadata,
            stale_github,
            order,
        })
    }

    pub fn order(&self) -> RowOrder {
        self.order
    }

    pub fn console_order(&self) -> RowOrder {
        // The CLI orders reservations by start. The Console also accepts
        // an optional end time on a received reservation.
        if self.command.source == Source::Reservation {
            self.command.console().order()
        } else {
            self.order
        }
    }

    pub fn lands_from_needs_you(&self) -> bool {
        self.metadata.landing && self.metadata.group == Group::NeedsYou
    }

    pub fn landing_kind(&self) -> Option<String> {
        self.lands_from_needs_you().then(|| {
            self.metadata.needs_you_kind.map_or_else(
                || "Needs you".into(),
                |kind| kind.title().to_ascii_lowercase(),
            )
        })
    }

    pub fn ready_member(&self) -> super::ready::Member<'_> {
        let (repository, pull_request, account) = match self.command.cli() {
            Cli::Record(record) => (
                record.facts.repository.as_str(),
                record.facts.pull_request.as_deref().unwrap_or_default(),
                record.view.account.as_deref(),
            ),
            Cli::History(history) => (history.repository.as_str(), "", history.account.as_deref()),
            Cli::Reservation(row) => (row.repository.as_str(), "", None),
        };
        super::ready::Member {
            repository,
            pull_request,
            order: self.order(),
            machine: &self.metadata.machine,
            account,
            selector: self.command.selector(),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct Metadata {
    #[serde(rename = "machine_name")]
    pub machine: String,
    pub group: Group,
    pub needs_you_kind: Option<NeedsYouKind>,
    pub landing: bool,
    pub stopping: bool,
    pub count: usize,
    #[serde(flatten)]
    pub github_checks: PullRequestChecks,
    #[serde(flatten)]
    pub departure: Option<Departure>,
    pub dismissal: Option<DismissalView>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Record,
    History,
    Reservation,
}

impl Source {
    fn read(value: &Value) -> Self {
        match value["source"].as_str() {
            Some("command_record") => Self::Record,
            Some("reservation") => Self::Reservation,
            _ => Self::History,
        }
    }
}

/// A single tree controls both omission and serialization. The optional arrays
/// keep the distinction between a missing field and a present, empty array.
#[derive(Clone, Serialize)]
pub(super) struct Command {
    #[serde(flatten)]
    fields: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    dispatched: Option<Vec<Self>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<Self>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    collapsed: Option<Vec<Self>>,
    #[serde(skip)]
    pub source: Source,
    #[serde(skip)]
    other_account: bool,
    #[serde(skip)]
    selector: String,
    #[serde(skip)]
    selector_label: String,
    #[serde(skip)]
    pub ready_title: Option<String>,
    #[serde(skip)]
    cli: Option<Arc<Cli>>,
    #[serde(skip)]
    console: Option<Arc<Console>>,
}

impl Command {
    fn read(mut fields: Value, cli: bool) -> Result<Self> {
        let source = Source::read(&fields);
        let other_account = !fields["account"].is_null();
        let path = fields["record_path"]
            .as_str()
            .or_else(|| fields["command_log"].as_str());
        let selector = selector(&fields, source);
        let selector_label = Self::selector_label_for(&selector, path).to_owned();
        let ready_title =
            super::ready::title(&fields, fields["pull_request"].as_str().unwrap_or_default())
                .map(str::to_owned);
        let cli = cli
            .then(|| Cli::read(&fields, source))
            .transpose()?
            .map(Arc::new);
        let cli_children = cli.is_some() && source == Source::Record;
        let object = fields.as_object_mut().context("invalid Command")?;
        let mut read_children = |field, cli| -> Result<_> {
            object
                .remove(field)
                .map(|value| {
                    let Value::Array(children) = value else {
                        anyhow::bail!("invalid Command tree");
                    };
                    children
                        .into_iter()
                        .map(|child| Self::read(child, cli))
                        .collect()
                })
                .transpose()
        };
        Ok(Self {
            dispatched: read_children("dispatched", cli_children)?,
            children: read_children("children", cli_children)?,
            collapsed: read_children("collapsed", false)?,
            fields,
            source,
            other_account,
            selector,
            selector_label,
            ready_title,
            cli,
            console: None,
        })
    }

    pub fn local_console(value: Value) -> Result<Self> {
        let mut command = Self::read(value, false)?;
        command.check_console()?;
        Ok(command)
    }

    fn check_console(&mut self) -> Result<()> {
        self.console = Some(Arc::new(Console::read(
            &self.fields,
            self.source,
            &self.selector,
        )?));
        for child in self
            .dispatched
            .iter_mut()
            .flatten()
            .chain(self.children.iter_mut().flatten())
        {
            child.check_console()?;
        }
        Ok(())
    }

    fn keep(&mut self, selector: &str) -> bool {
        if self.selector == selector {
            return false;
        }
        for branch in [
            &mut self.dispatched,
            &mut self.children,
            &mut self.collapsed,
        ]
        .into_iter()
        .flatten()
        {
            branch.retain_mut(|child| child.keep(selector));
        }
        true
    }

    pub fn selector(&self) -> &str {
        &self.selector
    }

    pub fn selector_label(&self) -> &str {
        &self.selector_label
    }

    pub fn selector_label_for<'a>(selector: &'a str, storage_path: Option<&str>) -> &'a str {
        // The selector is opaque and its Command name can contain slashes.
        storage_path
            .and_then(|path| path.rsplit_once('/'))
            .and_then(|(directory, _)| selector.strip_prefix(directory))
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(selector)
    }

    pub fn cli(&self) -> &Cli {
        self.cli
            .as_deref()
            .expect("CLI-visible Command was checked")
    }

    pub fn console(&self) -> &Console {
        self.console
            .as_deref()
            .expect("Console-visible Command was checked")
    }

    pub fn visible_children(&self) -> impl Iterator<Item = &Self> {
        self.dispatched
            .iter()
            .flatten()
            .chain(self.children.iter().flatten())
    }

    pub fn dispatched_count(&self) -> usize {
        self.dispatched.as_ref().map_or(0, Vec::len)
    }

    pub fn collapsed_are_owned_records(&self) -> bool {
        self.collapsed
            .iter()
            .flatten()
            .all(|child| child.source == Source::Record && !child.other_account)
    }
}

fn selector(value: &Value, source: Source) -> String {
    if let Some(selector) = value["selector"].as_str() {
        return selector.to_owned();
    }
    let path = value["record_path"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            let log = value["command_log"].as_str()?;
            Some(if source == Source::Record {
                Path::new(log)
                    .with_extension("command.json")
                    .to_string_lossy()
                    .into_owned()
            } else {
                log.to_owned()
            })
        });
    match (path, value["name"].as_str().filter(|name| !name.is_empty())) {
        (Some(path), Some(name)) => format!("{path}::{name}"),
        (Some(path), None) => path,
        (None, _) => String::new(),
    }
}

pub(super) enum Cli {
    Record(Box<Record>),
    History(CommandSummary),
    Reservation(Reservation),
}

pub(super) struct Record {
    pub facts: CommandRecord,
    pub view: RecordView,
}

#[derive(Deserialize)]
pub(super) struct RecordView {
    pub account: Option<String>,
    pub state: String,
    pub stage: Option<CurrentStage>,
    pub health: Option<Health>,
}

#[derive(Deserialize)]
pub(super) struct CommandSummary {
    pub account: Option<String>,
    pub repository: String,
    pub kind: String,
    pub issue: Option<Issue>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Reservation {
    pub repository: String,
    pub started_at: String,
    #[serde(rename = "reservation", deserialize_with = "reservation_reference")]
    pub reference: String,
    pub blocking: String,
    pub owner_description: String,
    pub release_command: String,
}

fn reservation_reference<'de, D: serde::Deserializer<'de>>(input: D) -> Result<String, D::Error> {
    let value = Value::deserialize(input)?;
    Ok(text(&value["reference"]))
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

impl Cli {
    fn read(value: &Value, source: Source) -> Result<Self> {
        Ok(match source {
            Source::Record => {
                anyhow::ensure!(
                    value.get("children").is_some() && value.get("dispatched").is_some(),
                    "missing Command children"
                );
                Self::Record(Box::new(Record {
                    facts: CommandRecord::deserialize(value)?,
                    view: RecordView::deserialize(value)?,
                }))
            }
            Source::History => Self::History(CommandSummary::deserialize(value)?),
            Source::Reservation => Self::Reservation(Reservation::deserialize(value)?),
        })
    }

    pub fn state(&self) -> &str {
        match self {
            Self::Record(record) => &record.view.state,
            Self::History(_) => "unknown",
            Self::Reservation(_) => "held",
        }
    }

    fn occurrence(&self) -> Option<&str> {
        match self {
            Self::Record(record) => record
                .facts
                .ended_at
                .as_deref()
                .or(Some(&record.facts.started_at)),
            Self::History(row) => row.ended_at.as_deref().or(Some(&row.started_at)),
            Self::Reservation(row) => Some(&row.started_at),
        }
    }
}

/// Console fields also apply to older History and reservation rows, and to
/// children with no Inbox metadata. These rows have their own acceptance rules.
#[derive(Deserialize)]
pub(super) struct Console {
    #[serde(flatten)]
    pub summary: CommandSummary,
    #[serde(default)]
    pub name: String,
    pub state: String,
    pub cause: Option<String>,
    pub pull_request: Option<String>,
    pub release_url: Option<String>,
    pub base_branch: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub stage: Option<CurrentStage>,
    pub health: Option<Health>,
    #[serde(default)]
    pub waiting_for: Vec<String>,
    pub resume_command: Option<String>,
    pub process_name: Option<String>,
    #[serde(flatten)]
    pub github_checks: PullRequestChecks,
    #[serde(default)]
    pub tickets: Vec<TicketRecord>,
    #[serde(default)]
    pub security_findings: Vec<crate::security::RecordedFinding>,
    #[serde(flatten)]
    pub departure: Option<Departure>,
    pub dismissal: Option<DismissalView>,
    #[serde(skip)]
    pub group: Option<Group>,
    #[serde(skip)]
    pub needs_you_kind: Option<NeedsYouKind>,
    #[serde(skip)]
    pub stopping: bool,
    #[serde(skip)]
    pub has_parent: bool,
    #[serde(skip)]
    pub end_time_estimated: bool,
    #[serde(skip)]
    pub count: usize,
    #[serde(skip)]
    pub reservation: Option<ReservationDisplay>,
    #[serde(skip)]
    pub release_command: Option<String>,
    #[serde(skip)]
    pub id: String,
}

pub(super) struct ReservationDisplay {
    pub reference: String,
    pub blocking: String,
    pub owner_description: String,
}

impl Console {
    fn read(value: &Value, source: Source, selector: &str) -> Result<Self> {
        let mut facts = Self::deserialize(value)?;
        facts.group = value.get("group").map(Group::deserialize).transpose()?;
        facts.needs_you_kind = value
            .get("needs_you_kind")
            .and_then(|kind| NeedsYouKind::deserialize(kind).ok());
        facts.stopping = value["stopping"] == true;
        facts.has_parent = !value["parent_record"].is_null();
        facts.end_time_estimated = value["end_time_estimated"] == true;
        facts.count = value["count"].as_u64().unwrap_or(1) as usize;
        facts.reservation = (source == Source::Reservation).then(|| ReservationDisplay {
            reference: text(&value["reservation"]["reference"]),
            blocking: text(&value["blocking"]),
            owner_description: text(&value["owner_description"]),
        });
        facts.release_command = value["release_command"].as_str().map(str::to_owned);
        facts.id = identity(&format!(
            "{selector}\0{}\0{}",
            facts.summary.kind, facts.summary.started_at
        ));
        Ok(facts)
    }

    pub fn order(&self) -> RowOrder {
        self.group.unwrap_or(Group::History).row_order(
            self.needs_you_kind,
            self.summary
                .ended_at
                .as_deref()
                .or(Some(&self.summary.started_at)),
        )
    }

    pub fn waiting_for_dismissal(&self) -> bool {
        matches!(self.group, Some(Group::NeedsYou | Group::ReadyForReview))
    }

    pub fn recorded_finish(&self) -> Option<&str> {
        self.summary
            .ended_at
            .as_deref()
            .filter(|_| !self.end_time_estimated)
    }
}

pub(super) fn identity(value: &str) -> String {
    // Encode the exact storage identity: issue numbers and timestamps alone
    // can collide, and DOM IDs cannot contain whitespace.
    let encoded: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("command-{encoded}")
}

#[cfg(test)]
mod tests;
