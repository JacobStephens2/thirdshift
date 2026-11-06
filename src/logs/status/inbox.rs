//! Owned Inbox presentation for the Console, from the same observations as status.
use anyhow::Result;
use chrono::{DateTime, Local};
use std::path::Path;

use super::{age, ticket_text};
use crate::config::UserConfig;
use crate::logs::command_record::Issue;
use crate::logs::dismiss::Action as DismissAction;
use crate::logs::pass_starts::PassStart;
use crate::logs::reading::{Command, Entry, Group, Health, IdlePass, Observation, Snapshot};

pub(crate) struct Inbox {
    /// Outer Command trees with live work, independent of sections and rows.
    pub active_runs: usize,
    pub sections: Vec<Section>,
    pub idle_passes: Vec<IdlePass>,
    pub pass_starts: Vec<PassStart>,
    pub warning: Option<&'static str>,
    pub github_warning: Option<String>,
}

pub(crate) struct Section {
    pub group: &'static str,
    pub title: &'static str,
    pub rows: Vec<Row>,
    pub references: Vec<String>,
}

pub(crate) struct Row {
    pub machine: String,
    pub console: String,
    pub stale: bool,
    pub account: Option<String>,
    pub id: String,
    pub started_ms: i64,
    pub occurred_ms: i64,
    pub live: bool,
    pub logging: bool,
    pub log_growth: String,
    pub selector: String,
    pub can_dismiss: bool,
    pub logs_url: String,
    pub subject: String,
    /// The pull request or issue's `https://` address, which the subject links to.
    pub subject_url: Option<String>,
    pub title: String,
    pub state: String,
    pub now_line: String,
    pub timing: Timing,
    pub count: usize,
    pub cause: String,
    pub resume_command: Option<String>,
    pub fix_command: Option<String>,
    /// Why and when the row left Needs you or Ready for review, empty
    /// while it still waits or never waited.
    pub left: String,
    pub harness: String,
    pub pull_request: String,
    pub ci: Option<crate::github::InboxCi>,
    pub merge_conflict: bool,
    pub tickets: Vec<String>,
    pub children: Vec<Row>,
    pub findings: Vec<crate::security::RecordedFinding>,
    pub sessions: Vec<std::path::PathBuf>,
}

pub(crate) struct Timing {
    pub finish: Option<Finish>,
    pub age: String,
}

pub(crate) struct Finish {
    pub datetime: String,
    pub label: String,
}

impl Timing {
    fn new(live: bool, start: &str, end: Option<&str>, now: DateTime<Local>) -> Self {
        if live {
            return Self {
                finish: None,
                age: elapsed(start, now),
            };
        }
        let finish = end.and_then(|end| {
            DateTime::parse_from_rfc3339(end).ok().map(|time| Finish {
                datetime: end.into(),
                label: crate::local_time::stamp(&time),
            })
        });
        let age = finish.as_ref().map_or_else(
            || "End time unknown".into(),
            |finish| format!("{} ago", elapsed(&finish.datetime, now)),
        );
        Self { finish, age }
    }
}

impl Row {
    /// The one-line row's summary: where the Command stands and, for a
    /// Command without an issue title, the first thing it did or hit.
    pub fn gist(&self) -> String {
        let detail = if !self.title.is_empty() {
            None
        } else if let Some(child) = self.children.first() {
            Some(match child.subject.rsplit_once('#') {
                Some((_, number)) => format!("#{number} {}", child.title),
                None => child.subject.clone(),
            })
        } else if let Some(finding) = self.findings.first() {
            Some(finding.title.clone())
        } else {
            Some(self.cause.split_whitespace().collect::<Vec<_>>().join(" "))
        };
        match detail.filter(|detail| !detail.trim().is_empty() && *detail != self.now_line) {
            Some(detail) => format!("{} · {detail}", self.now_line),
            None => self.now_line.clone(),
        }
    }
    pub fn peek_url(&self) -> String {
        self.details_url("/peek", None)
    }
    pub fn log_url(&self) -> String {
        self.details_url("/log", None)
    }
    pub fn session_url(&self, session: &std::path::Path) -> String {
        self.details_url("/log", Some(session))
    }

    fn details_url(&self, route: &str, session: Option<&std::path::Path>) -> String {
        let url = super::inspect::url(route, &self.selector, session);
        if self.console.is_empty() {
            url
        } else {
            super::inspect::machine_url(&url, &self.machine)
        }
    }

    pub fn tag_machine(&mut self, name: &str, console: &str, stale: bool) {
        self.machine = name.into();
        self.console = console.trim_end_matches('/').into();
        self.stale = stale;
        self.id = identity(&format!("{name}\0{}", self.id));
        if !console.is_empty() {
            self.can_dismiss = false;
            self.logs_url = format!("{}/logs", self.console);
        }
        for child in &mut self.children {
            child.tag_machine(name, console, stale);
        }
    }
}

impl Inbox {
    /// Received rows keep the owning machine's grouping and presentation facts;
    /// this path never opens their paths or evaluates them against local GitHub.
    pub fn merge_remote(
        &mut self,
        document: &super::fleet::Document,
        entry: &crate::config::console::fleet::Entry,
        stale: bool,
    ) -> Result<()> {
        let rows = document
            .commands
            .iter()
            .map(|value| present(value, chrono::Local::now()))
            .collect::<Result<Vec<_>>>()?;
        let listed = rows
            .iter()
            .map(|row| row.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        for (value, mut row) in document.commands.iter().zip(rows) {
            let checked = super::text::Row::read(value)?;
            let Some(section) = self
                .sections
                .iter_mut()
                .find(|section| section.group == value["group"].as_str().unwrap_or_default())
            else {
                continue;
            };
            row.children.retain(|child| !listed.contains(&child.id));
            // `row.html` shows repository and issue number as the link text.
            // A received document chooses `issue.url` independently, so keep
            // the href only when it names that same GitHub issue or pull request.
            bind_subject_link(&mut row);
            row.tag_machine(&entry.name, &entry.address, stale);
            let subject = row.subject.clone();
            section.rows.push(row);
            // Local rows were already ordered; merge with the same Needs you
            // priorities and newest occurrence ordering, without cross-host collapse.
            section.rows.sort_by_key(|row| {
                (
                    if section.group == "needs you" {
                        match row.state.as_str() {
                            "Silent" => 0,
                            "Slow" => 1,
                            _ => 2,
                        }
                    } else {
                        0
                    },
                    std::cmp::Reverse(row.occurred_ms),
                )
            });
            if checked.landing && value["group"] == "needs you" {
                // The secondary Landing reference belongs beside all machines' rows.
                let reference = format!(
                    "[{}] {} · {} · see Needs you",
                    entry.name,
                    subject,
                    value["needs_you_kind"].as_str().unwrap_or("Needs you")
                );
                if let Some(landing) = self
                    .sections
                    .iter_mut()
                    .find(|section| section.group == "landing")
                {
                    landing.references.push(reference);
                }
            }
        }
        Ok(())
    }
}

/// Drop a subject href that is not the GitHub issue or pull request named
/// by the row's visible repository and number. The text stays either way.
fn bind_subject_link(row: &mut Row) {
    if !subject_link_matches(&row.subject, row.subject_url.as_deref()) {
        row.subject_url = None;
    }
    for child in &mut row.children {
        bind_subject_link(child);
    }
}

fn subject_link_matches(subject: &str, url: Option<&str>) -> bool {
    let Some(url) = url else {
        return true;
    };
    let label = subject
        .rsplit_once(" · ")
        .map(|(_, rest)| rest)
        .unwrap_or(subject);
    let Some((repository, number)) = label.rsplit_once('#') else {
        return false;
    };
    let Ok(number) = number.parse::<u64>() else {
        return false;
    };
    if let Ok(issue) = crate::issue::IssueUrl::parse(url) {
        return issue.repo_slug().eq_ignore_ascii_case(repository) && issue.number == number;
    }
    crate::review::PullRequestUrl::parse(url).is_ok_and(|pull| {
        pull.issue.repo_slug().eq_ignore_ascii_case(repository) && pull.issue.number == number
    })
}

pub(crate) fn validate_remote(document: &super::fleet::Document) -> Result<()> {
    for value in &document.commands {
        present(value, chrono::Local::now())?;
    }
    Ok(())
}

/// Pure presentation shared by local observations and received Commands.
/// Only the local caller attaches captured paths, activity and Session inventory.
fn present(value: &serde_json::Value, now: DateTime<Local>) -> Result<Row> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Facts {
        account: Option<String>,
        repository: String,
        kind: String,
        issue: Option<Issue>,
        #[serde(default)]
        name: String,
        state: String,
        started_at: String,
        ended_at: Option<String>,
        outcome: Option<String>,
        command_log: Option<std::path::PathBuf>,
        cause: Option<String>,
        pull_request: Option<String>,
        harness: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        stage: Option<crate::logs::reading::CurrentStage>,
        health: Option<Health>,
        resume_command: Option<String>,
        process_name: Option<String>,
        github_ci: Option<crate::github::InboxCi>,
        github_mergeable: Option<crate::github::InboxMergeable>,
        #[serde(default)]
        children: Vec<serde_json::Value>,
        #[serde(default)]
        dispatched: Vec<serde_json::Value>,
        #[serde(default)]
        tickets: Vec<crate::logs::command_record::TicketRecord>,
        #[serde(default)]
        security_findings: Vec<crate::security::RecordedFinding>,
        #[serde(flatten)]
        departure: Option<crate::logs::reading::Departure>,
        dismissal: Option<crate::logs::dismiss::DismissalView>,
    }
    let facts: Facts = serde_json::from_value(value.clone())?;
    let live = facts.state == "live";
    let selector = facts
        .command_log
        .as_ref()
        .map(|path| {
            let path = if value["source"] == "command_record" {
                path.with_extension("command.json")
            } else {
                path.clone()
            };
            if facts.name.is_empty() {
                path.to_string_lossy().into_owned()
            } else {
                format!("{}::{}", path.display(), facts.name)
            }
        })
        .unwrap_or_default();
    let process =
        (facts.kind == "Process").then(|| facts.process_name.as_deref().unwrap_or(&facts.name));
    let name = if value["source"] == "command_record" {
        &facts.name
    } else {
        &facts.kind
    };
    let (mut subject, mut title) = subject(&facts.repository, name, facts.issue.as_ref(), process);
    let mut subject_url = issue_url(facts.issue.as_ref());
    let ready = value["group"] == "ready for review";
    if ready && let Some(pr) = &facts.pull_request {
        subject = crate::review::PullRequestUrl::parse(pr)
            .map(|pr| format!("{}#{}", pr.issue.repo().slug(), pr.issue.number))
            .unwrap_or_else(|_| pr.clone());
        subject_url = Some(pr.clone()).filter(|url| url.starts_with("https://"));
        title = value["github"]["subjects"][pr]["facts"]["title"]
            .as_str()
            .unwrap_or("(title unavailable)")
            .into();
    }
    let group: Option<Group> = value
        .get("group")
        .map(|group| serde_json::from_value(group.clone()))
        .transpose()?;
    let kind = value
        .get("needs_you_kind")
        .and_then(|kind| serde_json::from_value(kind.clone()).ok());
    let state = group
        .map_or(facts.state.as_str(), |group| {
            group.state_word(kind, value["stopping"] == true, &facts.state)
        })
        .to_owned();
    Ok(Row {
        machine: String::new(),
        console: String::new(),
        stale: false,
        id: identity(&format!("{selector}\0{}\0{}", facts.kind, facts.started_at)),
        account: facts.account,
        started_ms: timestamp(&facts.started_at),
        occurred_ms: timestamp(facts.ended_at.as_deref().unwrap_or(&facts.started_at)),
        live,
        logging: live && !matches!(facts.health, Some(Health::Silent { .. })),
        log_growth: String::new(),
        selector,
        can_dismiss: false,
        logs_url: String::new(),
        subject,
        subject_url,
        title,
        state,
        now_line: super::stage_details(
            facts.outcome.as_deref(),
            &facts.state,
            facts.stage.as_ref(),
            facts.health.as_ref(),
        ),
        timing: Timing::new(live, &facts.started_at, facts.ended_at.as_deref(), now),
        count: value["count"].as_u64().unwrap_or(1) as usize,
        cause: facts.cause.unwrap_or_default(),
        resume_command: facts
            .resume_command
            .filter(|_| facts.outcome.as_deref() == Some("paused")),
        fix_command: None,
        left: facts
            .dismissal
            .as_ref()
            .map(|dismissal| dismissal.line())
            .into_iter()
            .chain(facts.departure.as_ref().map(|departure| departure.line()))
            .collect::<Vec<_>>()
            .join("; "),
        harness: format!(
            "{} · {} · {}",
            facts.harness.as_deref().unwrap_or("unknown Harness"),
            facts.model.as_deref().unwrap_or("default Model"),
            facts.effort.as_deref().unwrap_or("default Effort")
        ),
        pull_request: facts.pull_request.unwrap_or_default(),
        ci: ready.then(|| facts.github_ci.unwrap_or_default()),
        merge_conflict: ready
            && facts.github_mergeable == Some(crate::github::InboxMergeable::Conflicting),
        tickets: facts
            .tickets
            .iter()
            .map(|ticket| ticket_text(ticket, &facts.repository))
            .collect(),
        children: facts
            .dispatched
            .iter()
            .chain(&facts.children)
            .map(|child| present(child, now))
            .collect::<Result<_>>()?,
        findings: facts.security_findings,
        sessions: Vec::new(),
    })
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

pub(super) fn archive_from_read(
    config: &UserConfig,
    page: ArchivePage,
    outcome: &str,
    repository: &str,
    snapshot: &Snapshot,
    now: DateTime<Local>,
) -> Result<Archive> {
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
        .map(|entry| outcome_kind(facts(entry).1).to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let rows = entries
        .into_iter()
        .filter(|entry| {
            let (repo, result, _) = facts(entry);
            (repository.is_empty() || repository == repo)
                && (outcome.is_empty() || outcome == outcome_kind(result) || outcome == result)
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
        Observation::Record(command) => row(command, now, root),
        Observation::History(command) => {
            let (subject, title) = subject(
                &command.repository,
                &command.kind,
                command.issue.as_ref(),
                None,
            );
            Ok(Row {
                machine: String::new(),
                console: String::new(),
                stale: false,
                account: command.account.clone(),
                id: identity(&match &command.account {
                    Some(account) => format!(
                        "history:{account}:{}:{}:{}",
                        command.repository, command.kind, command.started_at
                    ),
                    None => format!(
                        "history:{}:{}:{}",
                        command.repository, command.kind, command.started_at
                    ),
                }),
                started_ms: timestamp(&command.started_at),
                occurred_ms: timestamp(command.ended_at.as_deref().unwrap_or(&command.started_at)),
                live: false,
                logging: false,
                log_growth: String::new(),
                selector: Observation::History(command).selector().unwrap_or_default(),
                can_dismiss: false,
                logs_url: command
                    .command_log
                    .as_ref()
                    .map(|path| logs_url(path, root, command.account.as_deref()))
                    .unwrap_or_else(|| "/?page=logs".into()),
                subject,
                subject_url: issue_url(command.issue.as_ref()),
                title,
                state: command.state.into(),
                now_line: command
                    .outcome
                    .clone()
                    .unwrap_or_else(|| "unknown outcome".into()),
                timing: Timing::new(false, &command.started_at, command.ended_at.as_deref(), now),
                count: 1,
                cause: String::new(),
                resume_command: None,
                fix_command: None,
                left: String::new(),
                harness: String::new(),
                pull_request: String::new(),
                ci: None,
                merge_conflict: false,
                tickets: Vec::new(),
                children: Vec::new(),
                findings: Vec::new(),
                sessions: Observation::History(command).sessions()?,
            })
        }
    }
}

pub(super) fn from_read(
    config: &UserConfig,
    snapshot: &Snapshot,
    kept: &crate::logs::github::Kept,
    now: DateTime<Local>,
) -> Result<Inbox> {
    let items = snapshot
        .evaluate_inbox(config.inbox_days, kept)
        .into_rows(false);
    let listed: std::collections::BTreeSet<_> = items
        .iter()
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
                    // Ready dispatched Runs have their own Inbox row, in
                    // whichever group they reached. Preserve the full tree in
                    // history/show without duplicating DOM IDs.
                    row.children
                        .retain(|child| !listed.contains(&child.selector));
                    row.state = item.state_word().into();
                    row.fix_command = item.fix_command();
                    if group == Group::ReadyForReview && !row.pull_request.is_empty() {
                        row.subject = crate::review::PullRequestUrl::parse(&row.pull_request)
                            .map(|pr| format!("{}#{}", pr.issue.repo().slug(), pr.issue.number))
                            .unwrap_or_else(|_| row.pull_request.clone());
                        row.subject_url = Some(row.pull_request.clone())
                            .filter(|url| url.starts_with("https://"));
                        row.title = kept
                            .pull_request(&row.selector, &row.pull_request)
                            .and_then(|pr| pr.title.as_deref())
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or("(title unavailable)")
                            .into();
                        row.ci = Some(item.github.ci.unwrap_or_default());
                        row.merge_conflict = item.github.mergeable
                            == Some(crate::github::InboxMergeable::Conflicting);
                    }
                    row.count = item.count;
                    row.cause = item.cause().unwrap_or_default().into();
                    row.can_dismiss = DismissAction::for_row(item) == Some(DismissAction::Dismiss);
                    row.left = item
                        .dismissal
                        .as_ref()
                        .map(|dismissal| dismissal.line())
                        .into_iter()
                        .chain(item.left_line())
                        .collect::<Vec<_>>()
                        .join("; ");
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
                    let (subject, _) = row_subject(&command.record);
                    let subject = match &command.account {
                        Some(account) => format!("[{account}] {subject}"),
                        None => subject,
                    };
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
                rows,
                references,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Inbox {
        active_runs: snapshot.active_runs(),
        sections,
        github_warning: kept.warning(),
        idle_passes: snapshot.idle_passes.clone(),
        pass_starts: snapshot.pass_starts.clone(),
        warning: (config.effective_waiting_line_interval() != config.waiting_line_interval).then_some(
            "logs.waiting_line_minutes must be less than inbox.silent_minutes; using half the silent time for waiting lines"),
    })
}

fn row_subject(facts: &crate::logs::command_record::CommandRecord) -> (String, String) {
    subject(
        &facts.repository,
        &facts.name,
        facts.issue.as_ref(),
        facts.process_label(),
    )
}

pub(crate) fn subject(
    repository: &str,
    name: &str,
    issue: Option<&Issue>,
    process: Option<&str>,
) -> (String, String) {
    let (subject, title) = match issue {
        Some(issue) => (
            format!("{repository}#{}", issue.number),
            issue
                .title
                .clone()
                .unwrap_or_else(|| "(title unavailable)".into()),
        ),
        None => (
            format!("{} on {repository}", crate::logs::display_name(name)),
            String::new(),
        ),
    };
    let subject = match (process, issue) {
        (Some(process), Some(_)) => format!("Process {process} · {subject}"),
        (Some(process), None) => format!("Process {process} on {repository}"),
        (None, _) => subject,
    };
    (subject, title)
}

pub(crate) fn issue_url(issue: Option<&Issue>) -> Option<String> {
    issue
        .map(|issue| issue.url.clone())
        .filter(|url| url.starts_with("https://"))
}

fn row(command: &Command, now: DateTime<Local>, root: &Path) -> Result<Row> {
    fn attach_local(
        row: &mut Row,
        command: &Command,
        now: DateTime<Local>,
        root: &Path,
    ) -> Result<()> {
        let facts = &command.record;
        let (log_length, session_length) = command.activity_lengths();
        // A Pass's dispatched Runs share its storage path. Their immutable
        // Command facts distinguish them without tying identity to ordering.
        row.id = identity(&format!(
            "{}\0{}\0{}\0{}\0{}",
            command.record_path.to_string_lossy(),
            facts.kind,
            facts.name,
            facts.issue.as_ref().map_or("", |issue| issue.url.as_str()),
            facts.started_at
        ));
        row.log_growth = format!("{log_length}:{session_length}");
        row.selector = Observation::Record(command).selector().unwrap_or_default();
        row.logs_url = logs_url(&command.record_path, root, command.account.as_deref());
        row.timing = Timing::new(
            row.live,
            &facts.started_at,
            facts
                .ended_at
                .as_deref()
                .filter(|_| !facts.end_time_estimated),
            now,
        );
        row.sessions = command.sessions()?;
        for (child, command) in row
            .children
            .iter_mut()
            .zip(command.dispatched.iter().chain(&command.children))
        {
            attach_local(child, command, now, root)?;
        }
        Ok(())
    }
    let mut row = present(&serde_json::to_value(command)?, now)?;
    attach_local(&mut row, command, now, root)?;
    Ok(row)
}

fn logs_url(path: &std::path::Path, root: &std::path::Path, account: Option<&str>) -> String {
    let relative = if account.is_some() {
        Ok(path)
    } else {
        path.strip_prefix(root)
    };
    relative
        .map(|path| format!("/logs/command/{}", crate::logs::catalog::id(path)))
        .unwrap_or_else(|_| "/?page=logs".into())
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

/// What kind of outcome `text` is, for filtering: `merged`, `ready for
/// review`, `failed`, `plan` or `no Strong candidate`, without the PR,
/// issue or error it names. Other outcomes, such as `interrupted`, are
/// their own kind.
fn outcome_kind(text: &str) -> &str {
    if text.starts_with("failed") || text.ends_with(" failed") {
        "failed"
    } else if text == "merged" || (text.starts_with("PR ") && text.ends_with(" is merged")) {
        "merged"
    } else if text.ends_with("ready for review") {
        "ready for review"
    } else if text == "plan published" || text.starts_with("plan ") {
        "plan"
    } else if text.starts_with("no Strong candidate") {
        "no Strong candidate"
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::outcome_kind;

    #[test]
    fn outcome_kinds_drop_the_pr_issue_or_error_they_name() {
        for (text, kind) in [
            (
                "PR https://github.com/acme/widgets/pull/5 is merged",
                "merged",
            ),
            ("merged", "merged"),
            (
                "PR https://github.com/acme/widgets/pull/6 is ready for review",
                "ready for review",
            ),
            ("ready for review", "ready for review"),
            ("failed", "failed"),
            ("failed: claude exited 1", "failed"),
            (
                "failed: PR https://github.com/acme/widgets/pull/7 is merged, not open",
                "failed",
            ),
            ("review failed", "failed"),
            (
                "plan https://github.com/acme/widgets/issues/8 dispatched 3 Tickets",
                "plan",
            ),
            ("plan published", "plan"),
            (
                "no Strong candidate: the Architecture review filed the idea",
                "no Strong candidate",
            ),
            ("interrupted", "interrupted"),
        ] {
            assert_eq!(outcome_kind(text), kind, "{text}");
        }
    }

    /// `row.html` puts `subject_url` in the subject link while the visible
    /// text is the row's repository and issue number. A Fleet document chooses
    /// those independently, so a mismatched URL must not become that href.
    #[test]
    fn a_fleet_issue_link_stays_on_the_repository_the_row_names() {
        use crate::config::console::fleet::Entry;

        let command = |title: &str, url: &str, child: &str| {
            serde_json::json!({
                "machine_name": "remote",
                "group": "needs you",
                "landing": false,
                "stopping": false,
                "count": 1,
                "repository": "acme/widgets",
                "kind": "Run",
                "name": "fixture",
                "state": "failed",
                "started_at": "2026-01-01T00:00:00Z",
                "issue": { "number": 1, "url": url, "title": title },
                "children": [{
                    "repository": "acme/widgets",
                    "kind": "Run",
                    "name": "child",
                    "state": "failed",
                    "started_at": "2026-01-02T00:00:00Z",
                    "issue": { "number": 2, "url": child, "title": "Child" }
                }]
            })
        };
        let document: crate::logs::status::fleet::Document = serde_json::from_value(serde_json::json!({
            "version": 2,
            "hostname": "remote-host",
            "active_runs": 0,
            "commands": [
                command("Off GitHub", "https://evil.example/phish", "https://github.com/attacker/phish/issues/2"),
                command("Wrong number", "https://github.com/acme/widgets/issues/99", "https://github.com/acme/widgets/issues/2"),
                command("Matching", "https://github.com/acme/widgets/issues/1", "https://github.com/acme/widgets/issues/2"),
                {
                    "machine_name": "remote",
                    "group": "needs you",
                    "landing": false,
                    "stopping": false,
                    "count": 1,
                    "repository": "notacme/widgets",
                    "kind": "Run",
                    "name": "suffix",
                    "state": "failed",
                    "started_at": "2026-01-03T00:00:00Z",
                    "issue": {
                        "number": 1,
                        "url": "https://github.com/acme/widgets/issues/1",
                        "title": "Suffix"
                    }
                },
                {
                    "machine_name": "remote",
                    "group": "needs you",
                    "landing": false,
                    "stopping": false,
                    "count": 1,
                    "repository": "acme/widgets",
                    "kind": "Process",
                    "name": "build",
                    "process_name": "build",
                    "state": "failed",
                    "started_at": "2026-01-04T00:00:00Z",
                    "issue": {
                        "number": 1,
                        "url": "https://github.com/acme/widgets/issues/1",
                        "title": "Process"
                    }
                },
                {
                    "machine_name": "remote",
                    "group": "ready for review",
                    "landing": false,
                    "stopping": false,
                    "count": 1,
                    "repository": "ignored/repo",
                    "kind": "Run",
                    "name": "review",
                    "state": "done",
                    "started_at": "2026-01-05T00:00:00Z",
                    "pull_request": "https://github.com/acme/widgets/pull/7"
                }
            ],
            "machines": []
        }))
        .unwrap();
        let mut inbox = super::Inbox {
            active_runs: 0,
            sections: vec![
                super::Section {
                    group: "needs you",
                    title: "Needs you",
                    rows: Vec::new(),
                    references: Vec::new(),
                },
                super::Section {
                    group: "ready for review",
                    title: "Ready for review",
                    rows: Vec::new(),
                    references: Vec::new(),
                },
            ],
            idle_passes: Vec::new(),
            pass_starts: Vec::new(),
            warning: None,
            github_warning: None,
        };
        inbox
            .merge_remote(
                &document,
                &Entry {
                    name: "remote".into(),
                    address: "https://fleet.example".into(),
                    hostname: None,
                    thirdshift_version: None,
                    scopes: Vec::new(),
                    last_contact: None,
                },
                false,
            )
            .unwrap();
        let rows = &inbox.sections[0].rows;
        assert_eq!(rows.len(), 5);
        let row = |title: &str| {
            rows.iter()
                .find(|row| row.title == title)
                .unwrap_or_else(|| panic!("missing {title}"))
        };
        let off_github = row("Off GitHub");
        assert_eq!(off_github.subject, "acme/widgets#1");
        assert_eq!(off_github.subject_url, None, "off-GitHub URL");
        assert_eq!(off_github.children[0].subject, "acme/widgets#2");
        assert_eq!(off_github.children[0].subject_url, None, "other repository");
        let wrong_number = row("Wrong number");
        assert_eq!(wrong_number.subject, "acme/widgets#1");
        assert_eq!(
            wrong_number.subject_url, None,
            "same repository, different issue"
        );
        assert_eq!(
            wrong_number.children[0].subject_url.as_deref(),
            Some("https://github.com/acme/widgets/issues/2")
        );
        let matching = row("Matching");
        assert_eq!(
            matching.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/issues/1")
        );
        assert_eq!(
            matching.children[0].subject_url.as_deref(),
            Some("https://github.com/acme/widgets/issues/2")
        );
        assert_eq!(row("Suffix").subject, "notacme/widgets#1");
        assert_eq!(
            row("Suffix").subject_url,
            None,
            "repository is not a suffix of another"
        );
        assert_eq!(row("Process").subject, "Process build · acme/widgets#1");
        assert_eq!(
            row("Process").subject_url.as_deref(),
            Some("https://github.com/acme/widgets/issues/1")
        );
        let ready = &inbox.sections[1].rows;
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].subject, "acme/widgets#7");
        assert_eq!(
            ready[0].subject_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/7")
        );
    }
}
