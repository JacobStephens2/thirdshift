//! Owned Inbox presentation for the Console, from the same observations as status.
use anyhow::Result;
use chrono::{DateTime, Local};

use super::{age, stage_text, ticket_text};
use crate::config::UserConfig;
use crate::logs::command_record::Issue;
use crate::logs::reading::{Command, Entry, Group, Observation, Snapshot};

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
    pub selector: String,
    pub subject: String,
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
        .map(|entry| entry_row(entry, now))
        .collect::<Result<_>>()?;
    Ok(Archive {
        rows,
        repositories,
        outcomes,
    })
}

fn entry_row(entry: &Entry, now: DateTime<Local>) -> Result<Row> {
    match entry {
        Entry::Record(command) => row(command, now),
        Entry::History(command) => {
            let (subject, title) =
                subject(&command.repository, &command.kind, command.issue.as_ref());
            Ok(Row {
                selector: Observation::History(command).selector().unwrap_or_default(),
                subject,
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
    let sections = Group::ORDER
        .into_iter()
        .filter(|group| *group != Group::History)
        .map(|group| {
            let rows = items
                .iter()
                .filter(|item| item.group == group)
                .map(|item| {
                    let mut row = entry_row(item.command, now)?;
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
                    let Entry::Record(command) = item.command else {
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

fn row(command: &Command, now: DateTime<Local>) -> Result<Row> {
    let facts = &command.record;
    let (subject, title) = subject(&facts.repository, &facts.name, facts.issue.as_ref());
    Ok(Row {
        selector: Observation::Record(command).selector().unwrap_or_default(),
        subject,
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
            .map(|child| row(child, now))
            .collect::<Result<_>>()?,
        findings: facts.security_findings.clone(),
        sessions: command.sessions()?,
    })
}

fn elapsed(time: &str, now: DateTime<Local>) -> String {
    DateTime::parse_from_rfc3339(time)
        .map(|time| age(now.signed_duration_since(time).num_seconds().max(0)))
        .unwrap_or_else(|_| "unknown age".into())
}
