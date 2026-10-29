//! Owned Inbox presentation for the Console, from the same observations as status.
use anyhow::Result;
use chrono::{DateTime, Local};
use std::path::Path;

use super::{age, stage_text, ticket_text};
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
    /// The issue's `https://` address, which the subject links to.
    pub subject_url: Option<String>,
    pub title: String,
    pub state: String,
    pub now_line: String,
    pub timing: Timing,
    pub count: usize,
    pub cause: String,
    /// Why and when the row left Needs you or Ready for review, empty
    /// while it still waits or never waited.
    pub left: String,
    pub harness: String,
    pub pull_request: String,
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
    let snapshot = Snapshot::for_user(config, now)?;
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
                left: String::new(),
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
    let snapshot = Snapshot::for_user(config, now)?;
    // The Console shows the same groups as `status` by reading the same
    // kept answer. The Console's background reader refreshes it independently
    // of requests; status reads GitHub only when that answer is too old.
    let kept = crate::logs::github::load(&config.logs_dir);
    let items = snapshot
        .evaluate_inbox(config.inbox_days, &kept)
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
        idle_passes: snapshot.idle_passes,
        pass_starts: snapshot.pass_starts,
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
    let facts = &command.record;
    let (log_length, session_length) = command.activity_lengths();
    let live = command.state == "live";
    let (subject, title) = row_subject(facts);
    Ok(Row {
        account: command.account.clone(),
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
        occurred_ms: timestamp(facts.ended_at.as_deref().unwrap_or(&facts.started_at)),
        live,
        logging: live && !matches!(command.health, Some(Health::Silent { .. })),
        log_growth: format!("{log_length}:{session_length}"),
        selector: Observation::Record(command).selector().unwrap_or_default(),
        can_dismiss: false,
        logs_url: logs_url(&command.record_path, root, command.account.as_deref()),
        subject,
        subject_url: issue_url(facts.issue.as_ref()),
        title,
        state: command.state.into(),
        now_line: stage_text(command),
        timing: Timing::new(
            live,
            &facts.started_at,
            facts
                .ended_at
                .as_deref()
                .filter(|_| !facts.end_time_estimated),
            now,
        ),
        count: 1,
        cause: facts.cause.clone().unwrap_or_default(),
        left: String::new(),
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
            .map(|child| row(child, now, root))
            .collect::<Result<_>>()?,
        findings: facts.security_findings.clone(),
        sessions: command.sessions()?,
    })
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
}
