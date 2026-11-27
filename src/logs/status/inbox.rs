//! Owned Inbox presentation for the Console, from the same observations as status.
use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};
use std::path::Path;

use super::rows::{
    ConsoleCommand, ConsoleDocument, LocalCommand, Source, identity, selector_label,
};
use super::{age, ready, ticket_text};
use crate::config::UserConfig;
use crate::logs::command_record::Issue;
use crate::logs::dismiss::Action as DismissAction;
use crate::logs::github::PullRequestChecks;
use crate::logs::pass_starts::PassStart;
use crate::logs::reading::{Command, Entry, Group, Health, IdlePass, Observation, Snapshot};

pub(crate) struct Inbox {
    /// Outer Command trees with live work, independent of sections and rows.
    pub active_runs: usize,
    pub sections: Vec<Section>,
    pub idle_passes: Vec<IdlePass>,
    pub pass_starts: Vec<PassStart>,
    pub github_quota_wait: Option<String>,
    /// Every configured default route on this machine is unavailable by
    /// Plan usage, so scheduled Passes skip.
    pub plan_usage_banner: Option<String>,
    pub warning: Option<&'static str>,
    pub github_warning: Option<String>,
}

pub(crate) struct Section {
    pub group: &'static str,
    pub title: &'static str,
    pub rows: Vec<Row>,
    pub references: Vec<LandingReference>,
}

impl Section {
    const READY_FOR_REVIEW: &'static str = "ready for review";

    fn is_ready_for_review(&self) -> bool {
        self.group == Self::READY_FOR_REVIEW
    }

    fn display(&self) -> DisplaySection<'_> {
        let rows = if self.is_ready_for_review() {
            ready::group(self.rows.iter().collect(), |row| ready::Member {
                repository: &row.repository,
                pull_request: &row.pull_request,
                order: row.order,
                machine: &row.machine,
                account: row.account.as_deref(),
                selector: &row.selector,
            })
            .into_iter()
            .map(|group| match group.pull_request {
                Some(pr) => DisplayRow::PullRequest(pr, group.members),
                None => DisplayRow::Command(group.members[0]),
            })
            .collect()
        } else {
            self.rows.iter().map(DisplayRow::Command).collect()
        };
        DisplaySection {
            group: self.group,
            title: self.title,
            references: &self.references,
            rows,
        }
    }
}

pub(crate) struct DisplaySection<'a> {
    pub group: &'static str,
    pub title: &'static str,
    pub rows: Vec<DisplayRow<'a>>,
    pub references: &'a [LandingReference],
}

impl DisplaySection<'_> {
    pub fn is_ready_for_review(&self) -> bool {
        self.group == Section::READY_FOR_REVIEW
    }
}

pub(crate) enum DisplayRow<'a> {
    Command(&'a Row),
    PullRequest(ready::PullRequest, Vec<&'a Row>),
}

impl<'a> DisplayRow<'a> {
    /// The latest visible contributing Command supplies the pull request heading.
    pub fn heading(&self) -> &'a Row {
        match self {
            Self::Command(row) => row,
            Self::PullRequest(_, members) => members[0],
        }
    }

    pub fn completion_time(&self) -> Option<DateTime<FixedOffset>> {
        self.heading().timing.ended_or_started
    }

    pub fn needs_fix(&self) -> bool {
        match self {
            Self::Command(row) => row.needs_fix(),
            Self::PullRequest(_, members) => members.iter().any(|row| row.needs_fix()),
        }
    }
}

pub(crate) struct LandingReference {
    pub repository: String,
    pub text: String,
}

pub(crate) struct Row {
    order: crate::logs::reading::RowOrder,
    pub repository: String,
    pub machine: String,
    pub console: String,
    pub stale: bool,
    pub account: Option<String>,
    pub id: String,
    pub started_ms: i64,
    pub occurred_ms: i64,
    pub live: bool,
    pub logging: bool,
    pub slow: bool,
    pub log_growth: String,
    pub selector: String,
    pub selector_label: String,
    pub can_dismiss: bool,
    pub remote_stop: bool,
    pub remote_undo: bool,
    pub logs_url: String,
    pub subject: String,
    /// The pull request or issue's `https://` address, which the subject links to.
    pub subject_url: Option<String>,
    pub target_pr: Option<crate::review::PullRequestUrl>,
    pub base_branch: Option<String>,
    pub release_link: Option<ReleaseLink>,
    pub title: String,
    pub command_kind: String,
    pub state: String,
    pub now_line: String,
    pub timing: Timing,
    pub count: usize,
    pub cause: String,
    pub resume_command: Option<String>,
    pub fix_command: Option<String>,
    pub release_command: Option<String>,
    /// Why and when the row left Needs you or Ready for review, empty
    /// while it still waits or never waited.
    pub left: String,
    pub harness: String,
    pub pull_request: String,
    pub pr_checks: Option<PullRequestChecks>,
    pub stale_github: bool,
    pub ticket_progress: Option<TicketProgress>,
    pub tickets: Vec<String>,
    pub children: Vec<Row>,
    pub findings: Vec<crate::security::RecordedFinding>,
    pub sessions: Vec<std::path::PathBuf>,
}

/// Verified same-repository GitHub Release link shown beside a Release run's bump PR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReleaseLink {
    pub url: String,
    pub version: String,
}

impl ReleaseLink {
    fn parse(url: &str, repository: &str) -> Option<Self> {
        if url
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
            || !crate::github::valid_repo_slug(repository)
            || !super::target::is_safe_github_url(url)
        {
            return None;
        }
        let parsed = reqwest::Url::parse(url).ok()?;
        let segments: Vec<_> = parsed.path_segments()?.collect();
        let [owner, repo, "releases", "tag", tag] = segments.as_slice() else {
            return None;
        };
        if owner.is_empty()
            || repo.is_empty()
            || !format!("{owner}/{repo}").eq_ignore_ascii_case(repository)
        {
            return None;
        }
        let version = tag.strip_prefix('v')?;
        let valid_version = !version.is_empty()
            && version
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_digit())
            && !version.starts_with('.')
            && !version.ends_with('.')
            && !version.contains("..")
            && version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'));
        valid_version.then(|| Self {
            url: parsed.as_str().to_owned(),
            version: version.to_owned(),
        })
    }
}

/// Validate a Command's optional published release URL against its kind and repository.
pub(super) fn command_release_link(
    kind: &str,
    release_url: Option<&str>,
    repository: &str,
) -> Option<ReleaseLink> {
    (kind == "Release run")
        .then_some(release_url)
        .flatten()
        .and_then(|url| ReleaseLink::parse(url, repository))
}

#[derive(Clone, Copy)]
pub(crate) struct TicketProgress {
    pub complete: usize,
    pub total: usize,
}

pub(crate) struct Timing {
    pub finish: Option<Finish>,
    pub age: String,
    ended_or_started: Option<DateTime<FixedOffset>>,
}

pub(crate) struct Finish {
    pub datetime: String,
    pub label: String,
}

impl Timing {
    fn new(live: bool, start: &str, end: Option<&str>, now: DateTime<Local>) -> Self {
        let completion = end.filter(|_| !live).and_then(|end| {
            DateTime::parse_from_rfc3339(end)
                .ok()
                .map(|time| (end, time))
        });
        let finish = completion.map(|(end, time)| Finish {
            datetime: end.into(),
            label: crate::local_time::stamp(&time),
        });
        let age = match completion {
            Some((_, time)) => format!(
                "{} ago",
                age(now.signed_duration_since(time).num_seconds().max(0))
            ),
            None if live => elapsed(start, now),
            None => "End time unknown".into(),
        };
        Self {
            finish,
            age,
            ended_or_started: completion
                .map(|(_, time)| time)
                .or_else(|| DateTime::parse_from_rfc3339(start).ok()),
        }
    }
}

impl Row {
    fn needs_fix(&self) -> bool {
        self.pr_checks
            .as_ref()
            .is_some_and(PullRequestChecks::needs_fix)
    }

    fn present_needs_you_target(&mut self, kind: &str, repository: &str) {
        if !matches!(kind, "Review run" | "Triage run") {
            return;
        }
        self.set_pr_subject(repository);
    }

    fn present_release_target(&mut self, kind: &str, repository: &str) {
        if kind != "Release run" || self.release_link.is_none() {
            return;
        }
        self.set_pr_subject(repository);
    }

    fn set_pr_subject(&mut self, repository: &str) {
        let Some(pr) = super::target::validated_pr_url(&self.pull_request, repository) else {
            return;
        };
        self.subject = format!("{repository}#{}", pr.issue.number);
        self.subject_url = Some(pr.url);
        // The subject now carries the PR, including for a live problem row.
        self.target_pr = None;
    }

    fn present_ready(&mut self, title: Option<&str>, checks: &PullRequestChecks) {
        if self.pull_request.is_empty() {
            return;
        }
        let pull_request = crate::review::PullRequestUrl::parse(&self.pull_request).ok();
        self.subject = pull_request
            .as_ref()
            .map(|pr| format!("{}#{}", pr.issue.repo().slug(), pr.issue.number))
            .unwrap_or_else(|| self.pull_request.clone());
        self.subject_url =
            Some(self.pull_request.clone()).filter(|url| url.starts_with("https://"));
        self.title = title.unwrap_or("(title unavailable)").into();
        self.pr_checks = Some(checks.clone());
        self.fix_command = if checks.needs_fix() {
            pull_request.map(|pr| format!("thirdshift fix {}", pr.url))
        } else {
            None
        };
    }

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
        if self.release_command.is_some() {
            return String::new();
        }
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
            self.logs_url = format!("{}/logs", self.console);
        }
        for child in &mut self.children {
            child.tag_machine(name, console, stale);
        }
    }
}

impl Inbox {
    /// Project the visible sections after machine and repository selection.
    pub fn display_sections(&self) -> Vec<DisplaySection<'_>> {
        self.sections.iter().map(Section::display).collect()
    }

    pub(super) fn add_reservations(
        &mut self,
        values: &[serde_json::Value],
        now: DateTime<Local>,
    ) -> Result<()> {
        for value in values {
            let checked = LocalCommand::check(value.clone())?;
            let command = checked.console();
            let group = command.facts.named_group;
            if let Some(section) = self.sections.iter_mut().find(|section| {
                group.is_some_and(|group| section.group.eq_ignore_ascii_case(group.title()))
            }) {
                section.rows.push(present(command, now));
                section.rows.sort_by_key(|row| row.order);
            }
        }
        Ok(())
    }

    pub fn remote_actions(&mut self, machine: &str, allowed: bool) {
        fn apply(row: &mut Row, allowed: bool) {
            row.can_dismiss &= allowed;
            row.remote_stop &= allowed;
            row.remote_undo &= allowed;
            for child in &mut row.children {
                apply(child, allowed);
            }
        }
        for row in self
            .sections
            .iter_mut()
            .flat_map(|section| &mut section.rows)
        {
            if row.machine == machine && !row.console.is_empty() {
                apply(row, allowed);
            }
        }
    }

    /// Merge the owning machine's checked facts without reading local paths or GitHub.
    pub fn merge_remote(
        &mut self,
        document: &ConsoleDocument,
        entry: &crate::config::console::fleet::Entry,
        stale: bool,
        failed: bool,
    ) {
        let now = Local::now();
        let listed = document
            .rows()
            .map(|row| row.command.facts.id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for checked in document.rows() {
            let facts = checked.command.facts;
            let Some(group) = facts.named_group else {
                continue;
            };
            let Some(section) = self
                .sections
                .iter_mut()
                .find(|section| section.group.eq_ignore_ascii_case(group.title()))
            else {
                continue;
            };
            let mut row = present(checked.command, now);
            row.children
                .retain(|child| !listed.contains(child.id.as_str()));
            row.tag_machine(
                &entry.name,
                &entry.address,
                stale || (section.is_ready_for_review() && failed),
            );
            row.stale_github = checked.stale_github && section.is_ready_for_review();
            let reference = checked.landing_kind.map(|kind| LandingReference {
                repository: facts.fields.summary.repository.clone(),
                text: format!("[{}] {} · {kind} · see Needs you", entry.name, row.subject),
            });
            section.rows.push(row);
            // Keep the owning Inbox's order without collapsing across machines.
            section.rows.sort_by_key(|row| row.order);
            if let Some(reference) = reference
                && let Some(landing) = self
                    .sections
                    .iter_mut()
                    .find(|section| section.group == "landing")
            {
                landing.references.push(reference);
            }
        }
    }
}

/// Drop a subject href that is not the GitHub issue or pull request named
/// by the row's visible repository and number. The text stays either way.
fn bind_subject_link(row: &mut Row) {
    if !subject_link_matches(&row.subject, row.subject_url.as_deref()) {
        row.subject_url = None;
    }
}

pub(crate) fn subject_link_matches(subject: &str, url: Option<&str>) -> bool {
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

/// Render checked facts. The local caller adds activity, paths and Session inventory.
fn present(command: ConsoleCommand<'_>, now: DateTime<Local>) -> Row {
    let checked = command.facts;
    let facts = &checked.fields;
    let summary = &facts.summary;
    let live = facts.state == "live";
    let process =
        (summary.kind == "Process").then(|| facts.process_name.as_deref().unwrap_or(&facts.name));
    let live_target = live && matches!(summary.kind.as_str(), "Fix run" | "Review run");
    let name = if live_target {
        &summary.kind
    } else if command.source() == Source::Record {
        &facts.name
    } else {
        &summary.kind
    };
    let (subject, title) = subject(&summary.repository, name, summary.issue.as_ref(), process);
    let subject_url = issue_url(summary.issue.as_ref());
    let state = checked
        .group
        .map_or(facts.state.as_str(), |group| {
            group.state_word(checked.needs_you_kind, checked.stopping, &facts.state)
        })
        .to_owned();
    let release_link = command_release_link(
        &summary.kind,
        facts.release_url.as_deref(),
        &summary.repository,
    );
    let mut row = Row {
        repository: summary.repository.clone(),
        order: checked.order,
        machine: String::new(),
        console: String::new(),
        stale: false,
        id: checked.id.clone(),
        account: summary.account.clone(),
        started_ms: timestamp(&summary.started_at),
        occurred_ms: timestamp(summary.ended_at.as_deref().unwrap_or(&summary.started_at)),
        live,
        logging: live && !matches!(facts.health, Some(Health::Silent { .. })),
        slow: matches!(facts.health, Some(Health::Slow { .. })),
        log_growth: String::new(),
        selector_label: command.selector_label().to_owned(),
        selector: command.selector().to_owned(),
        can_dismiss: summary.account.is_none()
            && matches!(command.source(), Source::Record | Source::Reservation)
            && checked.waiting_for_dismissal()
            && command.collapsed_are_owned_records(),
        remote_stop: summary.account.is_none()
            && live
            && command.source() == Source::Record
            && !checked.has_parent
            && !checked.stopping,
        remote_undo: summary.account.is_none()
            && facts.dismissal.is_some()
            && !checked.waiting_for_dismissal(),
        logs_url: String::new(),
        subject,
        subject_url,
        target_pr: facts
            .pull_request
            .as_deref()
            .filter(|_| live_target)
            .and_then(|url| crate::review::PullRequestUrl::parse(url).ok())
            .filter(|pr| {
                pr.issue
                    .repo_slug()
                    .eq_ignore_ascii_case(&summary.repository)
            }),
        base_branch: facts.base_branch.clone().filter(|_| live_target),
        release_link,
        title,
        command_kind: process.map_or_else(
            || crate::logs::display_name(&summary.kind).to_owned(),
            |name| format!("Process {name}"),
        ),
        state,
        now_line: super::stage_details(
            summary.outcome.as_deref(),
            &facts.state,
            facts.stage.as_ref(),
            facts.health.as_ref(),
            &facts.waiting_for,
        ),
        timing: Timing::new(live, &summary.started_at, checked.recorded_finish(), now),
        count: checked.count,
        cause: facts.cause.clone().unwrap_or_default(),
        resume_command: facts
            .resume_command
            .clone()
            .filter(|_| summary.outcome.as_deref() == Some("paused")),
        fix_command: None,
        release_command: checked.release_command.clone(),
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
        pull_request: facts.pull_request.clone().unwrap_or_default(),
        pr_checks: None,
        stale_github: false,
        ticket_progress: (!facts.tickets.is_empty()).then(|| TicketProgress {
            complete: facts
                .tickets
                .iter()
                .filter(|ticket| {
                    matches!(
                        ticket.state,
                        crate::logs::command_record::TicketState::Merged
                    )
                })
                .count(),
            total: facts.tickets.len(),
        }),
        tickets: facts
            .tickets
            .iter()
            .map(|ticket| ticket_text(ticket, &summary.repository))
            .collect(),
        children: command
            .children()
            .map(|child| present(child, now))
            .collect(),
        findings: facts.security_findings.clone(),
        sessions: Vec::new(),
    };
    if let Some(reservation) = &checked.reservation {
        row.state = "Reservation".into();
        row.title = reservation.reference.clone();
        row.subject = summary.repository.clone();
        row.subject_url = None;
        row.now_line = format!(
            "{} · {}",
            reservation.blocking, reservation.owner_description
        );
        row.timing.age = format!("Held for {}", elapsed(&summary.started_at, now));
        row.harness.clear();
    }
    // Compact Inbox rows hide nested runs. Keep each Slow descendant's
    // stage and health visible on the row that represents its work.
    for child in row.children.iter().filter(|child| child.slow) {
        row.now_line
            .push_str(&format!(" · {}: {}", child.subject, child.now_line));
        row.slow = true;
    }
    // Compact Inbox rows hide nested runs. Carry the dispatched Spec's
    // recorded accounting onto its Pass without counting child attempts.
    if row.ticket_progress.is_none() {
        row.ticket_progress = row.children[..command.dispatched_count()]
            .iter()
            .rev()
            .find_map(|child| child.ticket_progress);
    }
    if checked.named_group == Some(Group::ReadyForReview) {
        row.present_ready(command.ready_title(), &facts.github_checks);
    } else if checked.group == Some(Group::NeedsYou) {
        row.present_needs_you_target(&summary.kind, &summary.repository);
    }
    row.present_release_target(&summary.kind, &summary.repository);
    // A record can name one repository and carry another repository's issue
    // URL. The visible subject stays; the href has to name that same record.
    bind_subject_link(&mut row);
    row
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

/// The History filters asked for. `commands` are selectors, as a Metrics
/// failure cluster chooses them: a row shows when it or a Command it owns is one.
pub(super) struct Chosen<'a> {
    pub(super) outcome: &'a str,
    pub(super) repository: &'a str,
    pub(super) commands: Option<&'a [String]>,
}

impl Chosen<'_> {
    fn names(&self, entry: &Entry) -> bool {
        fn owns(command: &Command, commands: &[String]) -> bool {
            Observation::Record(command)
                .selector()
                .is_some_and(|selector| commands.contains(&selector))
                || command
                    .children
                    .iter()
                    .chain(&command.dispatched)
                    .any(|child| owns(child, commands))
        }
        let Some(commands) = self.commands else {
            return true;
        };
        match entry {
            Entry::Record(command) => owns(command, commands),
            Entry::History(command) => Observation::History(command)
                .selector()
                .is_some_and(|selector| commands.contains(&selector)),
        }
    }
}

pub(super) fn archive_from_read(
    config: &UserConfig,
    page: ArchivePage,
    chosen: &Chosen<'_>,
    snapshot: &Snapshot,
    now: DateTime<Local>,
) -> Result<Archive> {
    let Chosen {
        outcome,
        repository,
        ..
    } = *chosen;
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
                && chosen.names(entry)
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
            let mut row = Row {
                repository: command.repository.clone(),
                order: Group::History.row_order(
                    None,
                    command.ended_at.as_deref().or(Some(&command.started_at)),
                ),
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
                slow: false,
                log_growth: String::new(),
                selector: Observation::History(command).selector().unwrap_or_default(),
                selector_label: String::new(),
                can_dismiss: false,
                remote_stop: false,
                remote_undo: false,
                logs_url: command
                    .command_log
                    .as_ref()
                    .map(|path| logs_url(path, root, command.account.as_deref()))
                    .unwrap_or_else(|| "/?page=logs".into()),
                subject,
                subject_url: issue_url(command.issue.as_ref()),
                target_pr: None,
                base_branch: None,
                release_link: None,
                title,
                command_kind: crate::logs::display_name(&command.kind).to_owned(),
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
                release_command: None,
                left: String::new(),
                harness: String::new(),
                pull_request: String::new(),
                pr_checks: None,
                stale_github: false,
                ticket_progress: None,
                tickets: Vec::new(),
                children: Vec::new(),
                findings: Vec::new(),
                sessions: Observation::History(command).sessions()?,
            };
            bind_subject_link(&mut row);
            Ok(row)
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
    let stale_github =
        super::stale_github_answer(kept.age.map(|age| age.as_secs()), kept.failure.is_some());
    let sections = Group::ORDER
        .into_iter()
        .filter(|group| *group != Group::History)
        .map(|group| {
            let rows = items
                .iter()
                .filter(|item| item.group == group)
                .map(|item| {
                    let mut row = entry_row(item.command, now, &config.logs_dir)?;
                    row.order = item.order();
                    // Ready dispatched Runs have their own Inbox row, in
                    // whichever group they reached. Preserve the full tree in
                    // history/show without duplicating DOM IDs.
                    row.children
                        .retain(|child| !listed.contains(&child.selector));
                    row.state = item.state_word().into();
                    if group == Group::NeedsYou
                        && let Observation::Record(command) = item.command
                    {
                        row.present_needs_you_target(
                            &command.record.kind,
                            &command.record.repository,
                        );
                    }
                    if group == Group::ReadyForReview && !row.pull_request.is_empty() {
                        let title = super::target::preferred_pr_title(
                            kept.pull_request(&row.selector, &row.pull_request)
                                .and_then(|pr| pr.title.as_deref()),
                            match item.command {
                                Observation::Record(command) => {
                                    command.record.pull_request_title.as_deref()
                                }
                                Observation::History(_) => None,
                            },
                        );
                        row.present_ready(title, &item.github.checks);
                        row.stale_github = stale_github;
                    }
                    // Needs you may replace the subject with its pull request
                    // after `present` checked the recorded issue link.
                    bind_subject_link(&mut row);
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
                    Some(LandingReference {
                        repository: command.record.repository.clone(),
                        text: format!(
                            "{subject} · {} · see Needs you",
                            item.needs_you_kind.expect("Needs you has a kind").title()
                        ),
                    })
                })
                .collect();
            Ok(Section {
                group: match group {
                    Group::NeedsYou => "needs you",
                    Group::ReadyForReview => Section::READY_FOR_REVIEW,
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
        github_quota_wait: kept.quota_wait(),
        plan_usage_banner: crate::plan_usage::banner(config),
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
        row.selector_label = selector_label(&row.selector, command.record_path.to_str()).to_owned();
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
    let checked = LocalCommand::check(serde_json::to_value(command)?)?;
    let mut row = present(checked.console(), now);
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
    use super::super::rows::WireDocument;
    use super::{LocalCommand, Row, outcome_kind};

    fn present(
        value: &serde_json::Value,
        now: chrono::DateTime<chrono::Local>,
    ) -> anyhow::Result<Row> {
        let checked = LocalCommand::check(value.clone())?;
        Ok(super::present(checked.console(), now))
    }

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

    #[test]
    fn fleet_console_preserves_presentation_for_object_form_enums() {
        use crate::config::console::fleet::Entry;
        use serde_json::json;

        let input = json!({
            "version": 2, "hostname": "owner-host", "active_runs": 1,
            "commands": [
                {
                    "source": "legacy_log", "group": "needs you", "needs_you_kind": {"slow": null},
                    "landing": true, "stopping": false, "count": 1,
                    "selector": "/owner/canonical.log", "repository": "acme/canonical-control",
                    "kind": "Run", "state": "live", "started_at": "2026-11-26T10:00:00Z",
                    "children": [{
                        "group": {"ready for review": null}, "selector": "/owner/nested.log",
                        "repository": "acme/nested-control", "kind": "Run", "state": "done",
                        "started_at": "2026-11-26T10:00:00Z",
                        "pull_request": "https://github.com/acme/nested-control/pull/42",
                        "github_ci": "red"
                    }]
                },
                {
                    "source": "reservation", "group": {"needs you": null},
                    "landing": false, "stopping": false, "count": 1,
                    "selector": "/owner/wire-object.json", "repository": "acme/wire-object-marker",
                    "kind": "Run", "state": "live", "started_at": "2026-11-26T10:00:00Z",
                    "reservation": {"reference": "origin"}, "blocking": "Architect run",
                    "owner_description": "Held", "release_command": "thirdshift release-reservation"
                }
            ]
        });
        let document = serde_json::from_value::<WireDocument>(input)
            .unwrap()
            .admit_console("owner")
            .unwrap();
        let mut inbox = super::Inbox {
            active_runs: 0,
            sections: [("needs you", "Needs you"), ("landing", "Landing")]
                .into_iter()
                .map(|(group, title)| super::Section {
                    group,
                    title,
                    rows: Vec::new(),
                    references: Vec::new(),
                })
                .collect(),
            idle_passes: Vec::new(),
            pass_starts: Vec::new(),
            github_quota_wait: None,
            plan_usage_banner: None,
            warning: None,
            github_warning: None,
        };
        inbox.merge_remote(
            &document,
            &Entry::link("owner".into(), "https://owner.example.test".into()),
            false,
            false,
        );

        let [parent] = inbox.sections[0].rows.as_slice() else {
            panic!("only the row with a string group belongs in Needs you");
        };
        assert_eq!(parent.selector, "/owner/canonical.log");
        let [child] = parent.children.as_slice() else {
            panic!("the nested Command should still render");
        };
        assert_eq!(child.selector, "/owner/nested.log");
        assert!(child.pr_checks.is_none());
        let [reference] = inbox.sections[1].references.as_slice() else {
            panic!("the Needs you Command should still have a Landing reference");
        };
        assert!(reference.text.ends_with(" · Needs you · see Needs you"));
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
        let document = serde_json::from_value::<WireDocument>(serde_json::json!({
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
        .unwrap()
        .admit_console("remote")
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
            github_quota_wait: None,
            plan_usage_banner: None,
            warning: None,
            github_warning: None,
        };
        inbox.merge_remote(
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
            false,
        );
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

    #[test]
    fn a_published_release_row_links_both_its_bump_pr_and_release() {
        let now = chrono::Local::now();
        let make = |group: &str,
                    needs_you_kind: Option<&str>,
                    kind: &str,
                    name: &str,
                    pr: Option<&str>,
                    release_url: Option<&str>| {
            serde_json::json!({
                "source": "command_record",
                "repository": "acme/widgets",
                "kind": kind,
                "name": name,
                "group": group,
                "needs_you_kind": needs_you_kind,
                "state": if outcome_is_ended(group) { "ended" } else { "failed" },
                "started_at": "2026-11-15T08:00:00Z",
                "ended_at": "2026-11-15T08:05:00Z",
                "outcome": if group == "needs you" { "failed" } else { "merged" },
                "pull_request": pr,
                "release_url": release_url,
            })
        };
        fn outcome_is_ended(group: &str) -> bool {
            matches!(group, "done" | "resolved" | "history")
        }

        let done = present(
            &make(
                "done",
                None,
                "Release run",
                "release-20261115T080000",
                Some("https://github.com/acme/widgets/pull/18"),
                Some("https://github.com/acme/widgets/releases/tag/v0.2.0"),
            ),
            now,
        )
        .unwrap();
        assert_eq!(done.subject, "acme/widgets#18");
        assert_eq!(
            done.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/18")
        );
        assert_eq!(
            done.release_link,
            Some(super::ReleaseLink {
                url: "https://github.com/acme/widgets/releases/tag/v0.2.0".into(),
                version: "0.2.0".into(),
            })
        );
        assert_eq!(done.target_pr, None);

        let needs = present(
            &make(
                "needs you",
                Some("Failed"),
                "Release run",
                "release-20261115T070000",
                Some("https://github.com/acme/widgets/pull/17"),
                Some("https://github.com/acme/widgets/releases/tag/v0.1.9-rc.1+build.4"),
            ),
            now,
        )
        .unwrap();
        assert_eq!(needs.subject, "acme/widgets#17");
        assert_eq!(
            needs.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/17")
        );
        assert_eq!(
            needs.release_link,
            Some(super::ReleaseLink {
                url: "https://github.com/acme/widgets/releases/tag/v0.1.9-rc.1+build.4".into(),
                version: "0.1.9-rc.1+build.4".into(),
            })
        );

        for group in ["resolved", "history"] {
            let preserved = present(
                &make(
                    group,
                    None,
                    "Release run",
                    "release-20261115T063000",
                    Some("https://github.com/acme/widgets/pull/18"),
                    Some("https://github.com/acme/widgets/releases/tag/v0.2.0"),
                ),
                now,
            )
            .unwrap();
            assert_eq!(preserved.subject, "acme/widgets#18", "{group}");
            assert_eq!(
                preserved.subject_url.as_deref(),
                Some("https://github.com/acme/widgets/pull/18"),
                "{group}"
            );
            assert_eq!(
                preserved.release_link,
                Some(super::ReleaseLink {
                    url: "https://github.com/acme/widgets/releases/tag/v0.2.0".into(),
                    version: "0.2.0".into(),
                }),
                "{group}"
            );
        }

        let unlinked = present(
            &make(
                "done",
                None,
                "Release run",
                "release-20261115T060000",
                Some("https://github.com/acme/widgets/pull/16"),
                None,
            ),
            now,
        )
        .unwrap();
        assert_eq!(unlinked.subject, "release-20261115T060000 on acme/widgets");
        assert_eq!(unlinked.subject_url, None);
        assert_eq!(unlinked.release_link, None);

        let without_pr = present(
            &make(
                "needs you",
                Some("Failed"),
                "Release run",
                "release-20261115T050000",
                None,
                Some("https://github.com/acme/widgets/releases/tag/v0.1.8"),
            ),
            now,
        )
        .unwrap();
        assert_eq!(
            without_pr.subject,
            "release-20261115T050000 on acme/widgets"
        );
        assert_eq!(without_pr.subject_url, None);
        assert_eq!(
            without_pr.release_link,
            Some(super::ReleaseLink {
                url: "https://github.com/acme/widgets/releases/tag/v0.1.8".into(),
                version: "0.1.8".into(),
            })
        );

        let ticket = present(
            &serde_json::json!({
                "repository": "acme/widgets",
                "kind": "Run",
                "name": "7",
                "group": "done",
                "state": "ended",
                "started_at": "2026-11-15T08:00:00Z",
                "ended_at": "2026-11-15T08:05:00Z",
                "outcome": "merged",
                "issue": {
                    "number": 7,
                    "url": "https://github.com/acme/widgets/issues/7",
                    "title": "Fix bug"
                },
                "pull_request": "https://github.com/acme/widgets/pull/7",
                "release_url": "https://github.com/acme/widgets/releases/tag/v0.2.0"
            }),
            now,
        )
        .unwrap();
        assert_eq!(
            ticket.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/issues/7")
        );
        assert_eq!(ticket.release_link, None);
    }

    #[test]
    fn a_fleet_release_link_stays_on_the_repository_the_row_names() {
        use crate::config::console::fleet::Entry;

        let command = |name: &str, kind: &str, pr: Option<&str>, release_url: Option<&str>| {
            serde_json::json!({
                "machine_name": "remote",
                "record_path": format!("/tmp/{name}.command.json"),
                "group": "needs you",
                "landing": false,
                "stopping": false,
                "count": 1,
                "repository": "acme/widgets",
                "kind": kind,
                "name": name,
                "state": "failed",
                "started_at": "2026-11-15T08:00:00Z",
                "pull_request": pr,
                "release_url": release_url
            })
        };
        let mut commands = vec![
            command(
                "valid",
                "Release run",
                Some("https://github.com/acme/widgets/pull/18"),
                Some("https://github.com/acme/widgets/releases/tag/v0.2.0"),
            ),
            command(
                "case-match",
                "Release run",
                Some("https://github.com/Acme/Widgets/pull/19"),
                Some("https://github.com/Acme/Widgets/releases/tag/v0.2.1"),
            ),
            command(
                "foreign-pr-and-release",
                "Release run",
                Some("https://github.com/attacker/widgets/pull/18"),
                Some("https://github.com/attacker/widgets/releases/tag/v0.2.0"),
            ),
            command(
                "valid-pr-foreign-release",
                "Release run",
                Some("https://github.com/acme/widgets/pull/22"),
                Some("https://github.com/attacker/widgets/releases/tag/v0.2.2"),
            ),
            command(
                "foreign-pr-valid-release",
                "Release run",
                Some("https://github.com/attacker/widgets/pull/23"),
                Some("https://github.com/acme/widgets/releases/tag/v0.2.3"),
            ),
            command(
                "non-release-kind",
                "Review run",
                Some("https://github.com/acme/widgets/pull/20"),
                Some("https://github.com/acme/widgets/releases/tag/v0.2.0"),
            ),
        ];
        for (index, bad) in [
            "http://github.com/acme/widgets/releases/tag/v0.2.0",
            "https://github.com/acme/widgets/releases/tag/v0.2.0/",
            "https://github.com/acme/widgets/releases/tag/v0.2.0?x=1",
            "https://github.com/acme/widgets/releases/tag/v0.2.0#frag",
            "https://github.com/acme/widgets/releases/download/v0.2.0/bin",
            "https://github.com/acme/widgets/releases/tag/v",
            "https://github.com/acme/widgets/releases/tag/latest",
            "https://github.com/acme/widgets/releases/tag/v0.2..0",
            "https://github.com/acme/widgets/releases/tag/v0.2.0.",
            "https://github.com/acme/widgets/releases/tag/v0.2\n.0",
            "https://github.com/acme/widgets/releases/tag/v0.2\u{1b}.0",
        ]
        .into_iter()
        .enumerate()
        {
            commands.push(command(
                &format!("bad-{index}"),
                "Release run",
                Some("https://github.com/acme/widgets/pull/21"),
                Some(bad),
            ));
        }

        let document = serde_json::from_value::<WireDocument>(serde_json::json!({
            "version": 2,
            "hostname": "remote-host",
            "active_runs": 0,
            "commands": commands,
            "machines": []
        }))
        .unwrap()
        .admit_console("remote")
        .unwrap();
        let mut inbox = super::Inbox {
            active_runs: 0,
            sections: vec![super::Section {
                group: "needs you",
                title: "Needs you",
                rows: Vec::new(),
                references: Vec::new(),
            }],
            idle_passes: Vec::new(),
            pass_starts: Vec::new(),
            github_quota_wait: None,
            plan_usage_banner: None,
            warning: None,
            github_warning: None,
        };
        inbox.merge_remote(
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
            false,
        );

        let rows = &inbox.sections[0].rows;
        let row = |marker: &str| {
            rows.iter()
                .find(|row| row.selector.contains(marker))
                .unwrap_or_else(|| panic!("missing {marker}"))
        };
        let valid = row("/valid.command.json");
        assert_eq!(
            valid.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/18")
        );
        assert_eq!(
            valid.release_link.as_ref().map(|link| link.url.as_str()),
            Some("https://github.com/acme/widgets/releases/tag/v0.2.0")
        );
        let case_match = row("/case-match.command.json");
        assert_eq!(
            case_match.subject_url.as_deref(),
            Some("https://github.com/Acme/Widgets/pull/19")
        );
        assert_eq!(
            case_match
                .release_link
                .as_ref()
                .map(|link| link.version.as_str()),
            Some("0.2.1")
        );
        let foreign = row("/foreign-pr-and-release.command.json");
        assert_eq!(foreign.subject_url, None);
        assert_eq!(foreign.release_link, None);
        let valid_pr_foreign_release = row("/valid-pr-foreign-release.command.json");
        assert_eq!(valid_pr_foreign_release.subject_url, None);
        assert_eq!(valid_pr_foreign_release.release_link, None);
        let foreign_pr_valid_release = row("/foreign-pr-valid-release.command.json");
        assert_eq!(foreign_pr_valid_release.subject_url, None);
        assert_eq!(
            foreign_pr_valid_release
                .release_link
                .as_ref()
                .map(|link| link.version.as_str()),
            Some("0.2.3")
        );
        let review = row("/non-release-kind.command.json");
        assert_eq!(
            review.subject_url.as_deref(),
            Some("https://github.com/acme/widgets/pull/20")
        );
        assert_eq!(review.release_link, None);
        for index in 0..9 {
            let bad = row(&format!("/bad-{index}.command.json"));
            assert_eq!(bad.subject_url, None, "bad-{index}");
            assert_eq!(bad.release_link, None, "bad-{index}");
        }
    }
}
