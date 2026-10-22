//! Owned Inbox presentation for the Console, from the same observations as status.
use anyhow::Result;
use chrono::{DateTime, Local};
use std::path::Path;

use super::{age, stage_text, ticket_text};
use crate::config::UserConfig;
use crate::logs::command_record::Issue;
use crate::logs::reading::{Command, Entry, Group, Health, Observation, Snapshot};

pub(crate) struct Inbox {
    pub sections: Vec<Section>,
    pub idle_passes: Vec<String>,
    pub warning: Option<&'static str>,
}

pub(crate) struct Section {
    pub group: &'static str,
    pub title: &'static str,
    pub open: bool,
    pub rows: Vec<Row>,
    pub references: Vec<String>,
}

pub(crate) struct Row {
    pub id: String,
    pub started_ms: i64,
    pub live: bool,
    pub logging: bool,
    pub log_growth: String,
    pub selector: String,
    pub logs_url: String,
    pub subject: String,
    /// The issue's `https://` address, which the subject links to.
    pub subject_url: Option<String>,
    pub title: String,
    pub state: String,
    pub now_line: String,
    pub age: String,
    pub count: usize,
    pub cause: String,
    pub harness: String,
    pub pull_request: String,
    pub tickets: Vec<String>,
    pub children: Vec<Row>,
    pub findings: Vec<crate::security::RecordedFinding>,
    pub sessions: Vec<std::path::PathBuf>,
}

impl Row {
    pub fn peek_url(&self) -> String {
        super::inspect::url("/peek", &self.selector, None)
    }
    pub fn log_url(&self) -> String {
        super::inspect::url("/log", &self.selector, None)
    }
    pub fn session_url(&self, session: &std::path::Path) -> String {
        super::inspect::url("/log", &self.selector, Some(session))
    }
}

pub(crate) struct Archive {
    pub rows: Vec<Row>,
    pub repositories: Vec<String>,
    pub outcomes: Vec<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ArchivePage {
    History,
    Security,
}

pub(crate) fn archive(
    config: &UserConfig,
    page: ArchivePage,
    outcome: &str,
    repository: &str,
) -> Result<Archive> {
    let now = Local::now();
    let snapshot = Snapshot::load_with_silence(&config.logs_dir, now, config.silent_interval)?;
    let mut entries: Vec<_> = snapshot
        .entries()
        .iter()
        .filter(|entry| match (page, entry) {
            (ArchivePage::History, Entry::Record(command)) => command.state != "live",
            (ArchivePage::History, Entry::History(_)) => true,
            (ArchivePage::Security, Entry::Record(command)) => {
                command.record.kind == "Security run"
            }
            (ArchivePage::Security, Entry::History(command)) => command.kind == "Security run",
        })
        .collect();
    fn facts(entry: &Entry) -> (&str, &str, &str) {
        match entry {
            Entry::Record(command) => (
                &command.record.repository,
                command.record.outcome.as_deref().unwrap_or(command.state),
                command
                    .record
                    .ended_at
                    .as_deref()
                    .unwrap_or(&command.record.started_at),
            ),
            Entry::History(command) => (
                &command.repository,
                command.outcome.as_deref().unwrap_or(command.state),
                command.ended_at.as_deref().unwrap_or(&command.started_at),
            ),
        }
    }
    entries
        .sort_by_key(|entry| std::cmp::Reverse(DateTime::parse_from_rfc3339(facts(entry).2).ok()));
    let repositories = entries
        .iter()
        .map(|entry| facts(entry).0.to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let outcomes = entries
        .iter()
        .map(|entry| facts(entry).1.to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let rows = entries
        .into_iter()
        .filter(|entry| {
            let (repo, result, _) = facts(entry);
            (repository.is_empty() || repository == repo)
                && (outcome.is_empty() || outcome == result)
        })
        .map(|entry| {
            entry_row(
                match entry {
                    Entry::Record(command) => Observation::Record(command),
                    Entry::History(command) => Observation::History(command),
                },
                now,
                &config.logs_dir,
            )
        })
        .collect::<Result<_>>()?;
    Ok(Archive {
        rows,
        repositories,
        outcomes,
    })
}

fn entry_row(entry: Observation<'_>, now: DateTime<Local>, root: &std::path::Path) -> Result<Row> {
    match entry {
        Observation::Record(command) => row(command, now, None, root),
        Observation::History(command) => {
            let (subject, title) =
                subject(&command.repository, &command.kind, command.issue.as_ref());
            Ok(Row {
                id: identity(&format!(
                    "history:{}:{}:{}",
                    command.repository, command.kind, command.started_at
                )),
                started_ms: timestamp(&command.started_at),
                live: false,
                logging: false,
                log_growth: String::new(),
                selector: Observation::History(command).selector().unwrap_or_default(),
                logs_url: command
                    .command_log
                    .as_ref()
                    .map(|path| logs_url(path, root))
                    .unwrap_or_else(|| "/?page=logs".into()),
                subject,
                subject_url: issue_url(command.issue.as_ref()),
                title,
                state: command.state.into(),
                now_line: command
                    .outcome
                    .clone()
                    .unwrap_or_else(|| "unknown outcome".into()),
                age: elapsed(
                    command.ended_at.as_deref().unwrap_or(&command.started_at),
                    now,
                ),
                count: 1,
                cause: String::new(),
                harness: String::new(),
                pull_request: String::new(),
                tickets: Vec::new(),
                children: Vec::new(),
                findings: Vec::new(),
                sessions: Observation::History(command).sessions()?,
            })
        }
    }
}

pub(crate) fn inbox(config: &UserConfig) -> Result<Inbox> {
    let now = Local::now();
    let snapshot = Snapshot::load_with_silence(&config.logs_dir, now, config.silent_interval)?;
    let items = snapshot.inbox(false, config.inbox_days);
    let ready_selectors: std::collections::BTreeSet<_> = items
        .iter()
        .filter(|item| item.group == Group::ReadyForReview)
        .filter_map(|item| item.command.selector())
        .collect();
    let sections = Group::ORDER
        .into_iter()
        .filter(|group| *group != Group::History)
        .map(|group| {
            let rows = items
                .iter()
                .filter(|item| item.group == group)
                .map(|item| {
                    let mut row = entry_row(item.command, now, &config.logs_dir)?;
                    // Ready dispatched Runs have their own Inbox row. Preserve
                    // the full tree in history/show without duplicating DOM IDs.
                    row.children
                        .retain(|child| !ready_selectors.contains(&child.selector));
                    if let Some(kind) = item.needs_you_kind {
                        row.state = kind.title().into();
                    }
                    row.count = item.count;
                    row.cause = item.cause().unwrap_or_default().into();
                    Ok(row)
                })
                .collect::<Result<_>>()?;
            let references = items
                .iter()
                .filter(|item| {
                    group == Group::Landing && item.landing && item.group == Group::NeedsYou
                })
                .filter_map(|item| {
                    let Observation::Record(command) = item.command else {
                        return None;
                    };
                    let (subject, _) = subject(
                        &command.record.repository,
                        &command.record.name,
                        command.record.issue.as_ref(),
                    );
                    Some(format!(
                        "{subject} · {} · see Needs you",
                        item.needs_you_kind.expect("Needs you has a kind").title()
                    ))
                })
                .collect();
            Ok(Section {
                group: match group {
                    Group::NeedsYou => "needs you",
                    Group::ReadyForReview => "ready for review",
                    Group::Landing => "landing",
                    Group::Working => "working",
                    Group::Done => "done",
                    Group::Resolved => "resolved",
                    Group::History => unreachable!(),
                },
                title: group.title(),
                open: group == Group::NeedsYou,
                rows,
                references,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Inbox {
        sections,
        idle_passes: snapshot.idle_passes.iter().map(|pass| format!("{} idle on {}: {}", pass.kind, pass.repository, pass.reason)).collect(),
        warning: (config.effective_waiting_line_interval() != config.waiting_line_interval).then_some(
            "logs.waiting_line_minutes must be less than inbox.silent_minutes; using half the silent time for waiting lines"),
    })
}

fn subject(repository: &str, name: &str, issue: Option<&Issue>) -> (String, String) {
    match issue {
        Some(issue) => (
            format!("{repository}#{}", issue.number),
            issue
                .title
                .clone()
                .unwrap_or_else(|| "(title unavailable)".into()),
        ),
        None => (format!("{name} on {repository}"), String::new()),
    }
}

fn issue_url(issue: Option<&Issue>) -> Option<String> {
    issue
        .map(|issue| issue.url.clone())
        .filter(|url| url.starts_with("https://"))
}

fn row(
    command: &Command,
    now: DateTime<Local>,
    outer_log: Option<&Path>,
    root: &Path,
) -> Result<Row> {
    let facts = &command.record;
    let log = outer_log.unwrap_or(&command.command_log);
    let live = command.state == "live";
    let (subject, title) = subject(&facts.repository, &facts.name, facts.issue.as_ref());
    Ok(Row {
        // A Pass's dispatched Runs share its storage path. Their immutable
        // Command facts distinguish them without tying identity to ordering.
        id: identity(&format!(
            "{}\0{}\0{}\0{}\0{}",
            command.record_path.to_string_lossy(),
            facts.kind,
            facts.name,
            facts.issue.as_ref().map_or("", |issue| issue.url.as_str()),
            facts.started_at
        )),
        started_ms: timestamp(&facts.started_at),
        live,
        logging: live && !matches!(command.health, Some(Health::Silent { .. })),
        log_growth: format!(
            "{}:{}",
            log_length(Some(log)),
            log_length(
                facts
                    .session_log
                    .as_deref()
                    .map(crate::logs::session::path)
                    .as_deref()
            )
        ),
        selector: Observation::Record(command).selector().unwrap_or_default(),
        logs_url: logs_url(&command.record_path, root),
        subject,
        subject_url: issue_url(facts.issue.as_ref()),
        title,
        state: command.state.into(),
        now_line: stage_text(command),
        age: elapsed(facts.ended_at.as_deref().unwrap_or(&facts.started_at), now),
        count: 1,
        cause: facts.cause.clone().unwrap_or_default(),
        harness: format!(
            "{} · {} · {}",
            facts.harness,
            facts.model.as_deref().unwrap_or("default Model"),
            facts.effort.as_deref().unwrap_or("default Effort")
        ),
        pull_request: facts.pull_request.clone().unwrap_or_default(),
        tickets: facts
            .tickets
            .iter()
            .map(|ticket| ticket_text(ticket, &facts.repository))
            .collect(),
        children: command
            .dispatched
            .iter()
            .chain(&command.children)
            .map(|child| row(child, now, Some(log), root))
            .collect::<Result<_>>()?,
        findings: facts.security_findings.clone(),
        sessions: command.sessions()?,
    })
}

fn logs_url(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .map(|path| format!("/logs/command/{}", crate::logs::catalog::id(path)))
        .unwrap_or_else(|_| "/?page=logs".into())
}

fn log_length(path: Option<&Path>) -> u64 {
    path.and_then(|path| std::fs::metadata(path).ok())
        .map_or(0, |file| file.len())
}

fn timestamp(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value).map_or(0, |time| time.timestamp_millis())
}

fn identity(value: &str) -> String {
    // Encode the exact storage identity: issue numbers and timestamps alone
    // can collide, and DOM IDs cannot contain whitespace.
    let encoded: String = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("command-{encoded}")
}

fn elapsed(time: &str, now: DateTime<Local>) -> String {
    DateTime::parse_from_rfc3339(time)
        .map(|time| age(now.signed_duration_since(time).num_seconds().max(0)))
        .unwrap_or_else(|_| "unknown age".into())
}
