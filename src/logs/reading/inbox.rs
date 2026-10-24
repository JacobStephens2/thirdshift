//! The machine's Inbox, computed from local observations for every reader.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, FixedOffset, Local};
use serde::Serialize;

use super::{Command, Entry, Health, Observation};
use crate::github::{PrState, PullRequestState};
use crate::logs::dismiss::{self, DismissalView, DismissedRow, READY_KIND};
use crate::logs::github::{FindingSubject, Kept, RowAnswer, Subject};
use crate::logs::stop;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::logs) enum Group {
    #[serde(rename = "needs you")]
    NeedsYou,
    #[serde(rename = "ready for review")]
    ReadyForReview,
    Landing,
    Working,
    Done,
    Resolved,
    History,
}

impl Group {
    pub(in crate::logs) const ORDER: [Self; 7] = [
        Self::NeedsYou,
        Self::ReadyForReview,
        Self::Landing,
        Self::Working,
        Self::Done,
        Self::Resolved,
        Self::History,
    ];

    pub(in crate::logs) fn title(self) -> &'static str {
        match self {
            Self::NeedsYou => "Needs you",
            Self::ReadyForReview => "Ready for review",
            Self::Landing => "Landing",
            Self::Working => "Working",
            Self::Done => "Done",
            Self::Resolved => "Resolved",
            Self::History => "History",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(in crate::logs) enum NeedsYouKind {
    Silent,
    Slow,
    Died,
    Failed,
    Triage,
    Finding,
}

impl NeedsYouKind {
    pub(in crate::logs) fn title(self) -> &'static str {
        match self {
            Self::Silent => "Silent",
            Self::Slow => "Slow",
            Self::Died => "Died",
            Self::Failed => "Failed",
            Self::Triage => "Triage",
            Self::Finding => "Finding",
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::Silent => 0,
            Self::Slow => 1,
            _ => 2,
        }
    }
}

/// Why a row left Needs you or Ready for review through the exit this
/// machine knows: a later Command starting on the same issue.
pub(in crate::logs) const TAKEN_UP_REASON: &str = "taken up by a later Command";
/// Why a waiting row left through GitHub: its pull request merged.
pub(in crate::logs) const MERGED_REASON: &str = "pull request merged";
/// Why a row left Ready for review through GitHub: its pull request closed.
pub(in crate::logs) const CLOSED_REASON: &str = "pull request closed";
pub(in crate::logs) const ISSUE_CLOSED_REASON: &str = "issue closed";

#[derive(Serialize)]
pub(in crate::logs) struct InboxItem<'a> {
    #[serde(flatten)]
    pub command: Observation<'a>,
    pub group: Group,
    pub needs_you_kind: Option<NeedsYouKind>,
    pub landing: bool,
    /// A live Command given a Stop, still ending.
    pub stopping: bool,
    pub count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collapsed: Vec<InboxItem<'a>>,
    /// A waiting group's departure, absent for rows that never waited or aged out.
    #[serde(flatten)]
    departure: Option<Departure>,
    /// What GitHub last showed for the row's pull request: `open`,
    /// `merged` or `closed`. `None` where GitHub speaks for no row: one
    /// with no pull request, one not read yet, or one GitHub can't resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_state: Option<String>,
    /// The pull request's merge or close time, RFC 3339, for a merged or
    /// closed row: when the row stopped waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_at: Option<String>,
    /// The row's own or dispatched issue/PR facts, finding waits and the
    /// time each was read. History retains its own observation times.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github: Option<RowAnswer>,
    /// Who dismissed the row, when, from where and all kinds covered.
    /// `None` while the row waits or was never dismissed for its kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissal: Option<DismissalView>,
}

#[derive(Serialize)]
struct Departure {
    left_from: String,
    left_reason: String,
    left_at: String,
}

impl Departure {
    fn line(&self) -> String {
        let Self {
            left_from,
            left_reason,
            left_at,
        } = self;
        if left_reason == TAKEN_UP_REASON {
            format!("left {left_from}: {left_reason} started at {left_at}")
        } else {
            format!("left {left_from}: {left_reason} at {left_at}")
        }
    }
}

impl InboxItem<'_> {
    pub(in crate::logs) fn cause(&self) -> Option<&str> {
        self.command.facts().cause
    }

    /// The line saying why and when the row left its waiting group, for
    /// `status` text and the Console row. `None` while it still waits or
    /// never waited.
    pub(in crate::logs) fn left_line(&self) -> Option<String> {
        self.departure.as_ref().map(Departure::line)
    }

    /// The row's state word in the Console and `status`: its Needs you kind,
    /// then Stopping, Ready or Landing, otherwise the Command's own state.
    pub(in crate::logs) fn state_word(&self) -> &str {
        if let Some(kind) = self.needs_you_kind {
            return kind.title();
        }
        if self.stopping {
            return "Stopping";
        }
        match self.group {
            Group::ReadyForReview => "Ready",
            Group::Landing => "Landing",
            _ => self.command.facts().state,
        }
    }
}

struct Facts<'a> {
    state: &'a str,
    started_at: &'a str,
    ended_at: Option<&'a str>,
    outcome: Option<&'a str>,
    cause: Option<&'a str>,
    ready: bool,
}

impl<'a> Observation<'a> {
    fn problem(self, stop: Option<&stop::Request>) -> Option<NeedsYouKind> {
        let facts = self.facts();
        if facts.state == "died" {
            Some(NeedsYouKind::Died)
        } else if matches!(self, Observation::Record(_) if facts.state == "ended" && !facts.ready && stop.is_some_and(|request| !request.requeue))
        {
            Some(NeedsYouKind::Triage)
        } else if facts
            .outcome
            .is_some_and(|outcome| outcome.contains("failed") || outcome == "review required")
            || matches!(self, Observation::Record(command) if command.record.kind == "Experiment" && matches!(facts.outcome, Some("fail" | "inconclusive" | "waiting_on_rechecks")))
        {
            Some(NeedsYouKind::Failed)
        } else if matches!(self, Observation::Record(command) if command.record.security_findings_waiting)
        {
            Some(NeedsYouKind::Finding)
        } else {
            None
        }
    }

    fn facts(self) -> Facts<'a> {
        match self {
            Self::Record(command) => Facts {
                state: command.state,
                started_at: &command.record.started_at,
                ended_at: command.record.ended_at.as_deref(),
                outcome: command.record.outcome.as_deref(),
                cause: command.record.cause.as_deref(),
                ready: command.record.outcome.as_deref() == Some("ready for review")
                    && command.record.pull_request.is_some(),
            },
            Self::History(command) => Facts {
                state: command.state,
                started_at: &command.started_at,
                ended_at: command.ended_at.as_deref(),
                outcome: command.outcome.as_deref(),
                cause: command
                    .outcome
                    .as_deref()
                    .and_then(|outcome| outcome.strip_prefix("failed: ")),
                ready: command.outcome.as_deref().is_some_and(|outcome| {
                    outcome.contains("PR https://") && outcome.ends_with("is ready for review")
                }),
            },
        }
    }
}

struct LocalFacts {
    stop: Option<stop::Request>,
    dismissals: dismiss::Captured,
}

/// Classification inputs for a row and the dispatched work that can answer it.
/// Side files are read once per owning path, even when several rows share it.
struct Captured<'a> {
    command: Observation<'a>,
    problem: Option<NeedsYouKind>,
    stopping: bool,
    dismissal: Option<DismissedRow>,
    dispatched: Vec<Captured<'a>>,
}

impl<'a> Captured<'a> {
    fn new(command: Observation<'a>, local: &mut BTreeMap<&'a Path, LocalFacts>) -> Self {
        let (request, dismissal) = match command {
            Observation::Record(record) => {
                let facts = local
                    .entry(&record.record_path)
                    .or_insert_with(|| LocalFacts {
                        stop: stop::requested(&record.record_path),
                        dismissals: dismiss::Captured::read(&record.record_path),
                    });
                (
                    facts.stop.as_ref(),
                    command
                        .selector()
                        .and_then(|row| facts.dismissals.covering(&row))
                        .cloned(),
                )
            }
            Observation::History(_) => (None, None),
        };
        let problem = command.problem(request);
        let stopping = command.facts().state == "live" && request.is_some();
        let dispatched = match command {
            Observation::Record(record) => record
                .dispatched
                .iter()
                .map(|run| Self::new(Observation::Record(run), local))
                .collect(),
            Observation::History(_) => Vec::new(),
        };
        Self {
            command,
            problem,
            stopping,
            dismissal,
            dispatched,
        }
    }

    /// A failed Pass is answered by its failed dispatched work; a Finding
    /// waits for its own private records rather than a successful fix.
    fn github_records(&self) -> Vec<&'a crate::logs::command_record::CommandRecord> {
        if self.problem == Some(NeedsYouKind::Finding) {
            return Vec::new();
        }
        let Observation::Record(command) = self.command else {
            return Vec::new();
        };
        if self.problem.is_some()
            && matches!(
                command.record.kind.as_str(),
                "Architect run" | "Security run"
            )
            && !self.dispatched.is_empty()
        {
            let failed: Vec<_> = self
                .dispatched
                .iter()
                .filter(|run| run.problem.is_some())
                .collect();
            return if failed.is_empty() {
                self.dispatched.iter().collect()
            } else {
                failed
            }
            .into_iter()
            .filter_map(|run| match run.command {
                Observation::Record(command) => Some(&command.record),
                Observation::History(_) => None,
            })
            .collect();
        }
        vec![&command.record]
    }
}

/// Owned work targets, independent of the kept answer so the adapter can apply
/// History freezes under its refresh lock without evaluating row policy again.
#[derive(Default)]
pub(in crate::logs) struct RefreshWork {
    pub subjects: Vec<Subject>,
    pub findings: Vec<FindingSubject>,
    pub freezes: Vec<Freeze>,
}

pub(in crate::logs) struct Freeze {
    pub row: String,
    pub url: String,
}

impl RefreshWork {
    pub(in crate::logs) fn has_remote_reads(&self) -> bool {
        !self.subjects.is_empty() || !self.findings.is_empty()
    }
}

struct Row<'a> {
    item: InboxItem<'a>,
    order: (Group, Option<u8>, Reverse<Option<DateTime<FixedOffset>>>),
    failure: Option<(&'a str, bool)>,
}

/// One operation's retained Inbox decisions. Projections read only this value;
/// a new local observation or kept GitHub answer requires a new evaluation.
pub(in crate::logs) struct Evaluation<'a> {
    rows: Vec<Row<'a>>,
    refresh: RefreshWork,
    next_change: Option<DateTime<Local>>,
}

impl<'a> Evaluation<'a> {
    pub(in crate::logs) fn refresh_work(&self) -> &RefreshWork {
        &self.refresh
    }

    pub(in crate::logs) fn next_change(&self) -> Option<DateTime<Local>> {
        self.next_change
    }

    /// The collapsed row containing an exact selector, with the same visible
    /// membership as the Inbox. An explicit History target remains undoable,
    /// but cannot pull visible Commands into that History row's Dismissal.
    pub(in crate::logs) fn into_target(mut self, selector: &str) -> Option<InboxItem<'a>> {
        let history = self
            .rows
            .iter()
            .find(|row| row.item.command.selector().as_deref() == Some(selector))?
            .item
            .group
            == Group::History;
        self.rows
            .retain(|row| (row.item.group == Group::History) == history);
        self.into_rows(true).into_iter().find(|item| {
            std::iter::once(item)
                .chain(&item.collapsed)
                .any(|member| member.command.selector().as_deref() == Some(selector))
        })
    }

    pub(in crate::logs) fn into_rows(self, all: bool) -> Vec<InboxItem<'a>> {
        // Filter before collapse: hidden History members cannot contribute to
        // visible counts or the Commands a Dismissal will cover.
        let mut rows: Vec<_> = self
            .rows
            .into_iter()
            .filter(|row| all || row.item.group != Group::History)
            .collect();
        rows.sort_by_key(|row| row.order);
        let mut collapsed: Vec<InboxItem<'_>> = Vec::new();
        let mut failures = BTreeMap::new();
        for row in rows {
            if let Some(key) = row.failure {
                if let Some(&index) = failures.get(&key) {
                    let first: &mut InboxItem<'_> = &mut collapsed[index];
                    first.count += 1;
                    first.collapsed.push(row.item);
                    continue;
                }
                failures.insert(key, collapsed.len());
            }
            collapsed.push(row.item);
        }
        collapsed
    }
}

pub(super) fn evaluate<'a>(
    entries: &'a [Entry],
    now: DateTime<Local>,
    days: i64,
    github: &Kept,
) -> Evaluation<'a> {
    // An accepted duration can exceed chrono's calendar. In that case every
    // representable timestamp is inside the requested retention period.
    let cutoff = now.checked_sub_signed(chrono::Duration::days(days));
    let mut local = BTreeMap::new();
    let captured: Vec<_> = entries
        .iter()
        .map(|entry| {
            Captured::new(
                match entry {
                    Entry::Record(command) => Observation::Record(command),
                    Entry::History(command) => Observation::History(command),
                },
                &mut local,
            )
        })
        .collect();
    let observations: Vec<_> = captured
        .iter()
        .flat_map(|outer| {
            std::iter::once(outer).chain(
                outer
                    .dispatched
                    .iter()
                    .filter(|run| run.command.facts().ready),
            )
        })
        .collect();
    let starts: Vec<_> = observations
        .iter()
        .map(|row| issue_starts(row.command))
        .collect();
    let starts_by_issue = all_starts(&starts);
    let mut latest = BTreeMap::new();
    for starts in &starts {
        for (issue, start) in starts {
            latest
                .entry(issue)
                .and_modify(|time: &mut DateTime<FixedOffset>| *time = (*time).max(*start))
                .or_insert(*start);
        }
    }
    let mut evaluation = Evaluation {
        rows: Vec::new(),
        refresh: RefreshWork::default(),
        next_change: None,
    };
    let mut subjects = BTreeMap::new();
    for (captured, starts) in observations.into_iter().zip(&starts) {
        let command = captured.command;
        let facts = command.facts();
        let ended = time(facts.ended_at);
        let recorded_dismissal = captured.dismissal.as_ref();
        let records = captured.github_records();
        let tree_problem = match command {
            Observation::Record(command) => live_problem(command),
            _ => None,
        };
        let live_problem = tree_problem.filter(|_| facts.state == "live");
        let urls = github_urls(command, captured.problem, tree_problem, &records);
        let finding_facts = command.selector().and_then(|row| github.findings(&row));
        let findings_waiting = matches!(command, Observation::Record(command) if command.record.security_findings_waiting)
            && finding_facts.is_none_or(|facts| facts.waiting());
        let github_settlement = github_left(captured, &records, github, ended);
        // Settling failed work removes that wait, while another finding
        // from the same audit can still need the Day shift.
        let finding_wait = github_settlement.is_some() && findings_waiting;
        let settled_at = github_settlement.as_ref().map(|(_, at)| *at);
        let mut problem = if finding_wait {
            Some(NeedsYouKind::Finding)
        } else {
            captured.problem
        };
        // Dismissing the failure does not dismiss the findings it hid.
        if problem == Some(NeedsYouKind::Failed)
            && findings_waiting
            && recorded_dismissal
                .as_ref()
                .is_some_and(|row| row.covers(NeedsYouKind::Failed.title()))
        {
            problem = Some(NeedsYouKind::Finding);
        }
        let landing = matches!(command, Observation::Record(command) if is_landing(command));
        let stopping = captured.stopping;
        // When this row stopped waiting through the later-Command exit:
        // the earliest later start on any of its issues. Only rows that
        // waited (ended problems and ready Commands) leave through it, so
        // a Command that never waited keeps going by when it ended, and a
        // row that ages out keeps going straight to History.
        let waited = problem.is_some() || facts.ready;
        let taken_up = if waited {
            superseded_at(starts, &starts_by_issue, ended)
        } else {
            None
        };
        // Age moves only rows GitHub can't speak for. An ended problem
        // with known facts stays until settlement; a ready row stays while
        // its pull request is open. A later local Command still takes up either.
        let github_moved = (taken_up.is_none() && !finding_wait)
            .then_some(github_settlement)
            .flatten();
        let github_holds = taken_up.is_none()
            && ((facts.ready
                && github_state(&command, github)
                    .is_some_and(|state| state.state == PrState::Open))
                || (problem.is_some_and(|kind| kind != NeedsYouKind::Finding)
                    && github_known(command, &records, github))
                || (problem == Some(NeedsYouKind::Finding) && finding_facts.is_some()));
        let held = (ended
            .or_else(|| time(Some(facts.started_at)))
            // A recent dismissal proves this row was still in the Inbox.
            // In particular, a long-running Command that dies has no end
            // timestamp: its old start must not hide its new Died kind.
            .into_iter()
            .chain(
                recorded_dismissal
                    .as_ref()
                    .and_then(|row| time(Some(&row.at))),
            )
            .max()
            .is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff))
            && !starts
                .iter()
                .any(|(issue, start)| latest.get(issue).is_some_and(|latest| latest > start)))
            || github_holds;
        // A dismissal holds for every kind it covered: a row that turns
        // into a kind it wasn't when dismissed waits again. Its kind is
        // independent of age or a later Command taking it up, so those
        // exits do not erase the dismissal or shorten its retention.
        let kind_now = live_problem
            .or(problem)
            .map(NeedsYouKind::title)
            .or(if facts.ready { Some(READY_KIND) } else { None });
        let dismissed_row =
            recorded_dismissal.filter(|row| kind_now.is_none_or(|kind| row.covers(kind)));
        let dismissed = dismissed_row.is_some();
        let dismissed_at = dismissed_row.as_ref().and_then(|row| time(Some(&row.at)));
        let dismissal = dismissed_row.as_ref().map(|row| row.view(kind_now));
        let needs_you_kind = live_problem
            .or_else(|| problem.filter(|_| held && github_moved.is_none()))
            .filter(|_| !dismissed);
        let from = if problem.is_some() {
            Some("needs you")
        } else if facts.ready {
            Some("ready for review")
        } else {
            None
        };
        let left = taken_up.filter(|_| from.is_some());
        // The later-Command exit wins where both apply. Reopening brings back only a row
        // still in Done or Resolved: History rows are not read again.
        let github_moved = github_moved.filter(|_| from.is_some());
        // A dismissal already dealt with the row. Later GitHub facts or
        // Commands may change its context, but cannot restart its retention.
        let resolved_at = if dismissed {
            dismissed_at
        } else {
            left.or_else(|| github_moved.as_ref().map(|(_, at)| *at))
                .or(ended)
        };
        let group = if needs_you_kind.is_some() {
            Group::NeedsYou
        } else if facts.ready && held && github_moved.is_none() && !dismissed {
            Group::ReadyForReview
        } else if landing {
            Group::Landing
        } else if facts.state == "live" {
            Group::Working
        } else if ended.is_some_and(|time| {
            now.checked_sub_signed(chrono::Duration::days(1))
                .is_none_or(|cutoff| time >= cutoff)
        }) {
            Group::Done
        } else if resolved_at
            .is_some_and(|at| cutoff.is_none_or(|cutoff| at.with_timezone(&Local) >= cutoff))
        {
            Group::Resolved
        } else {
            Group::History
        };
        // A row GitHub finished weeks ago goes straight to History when
        // first read, instead of filling Resolved at once.
        let state = github_state(&command, github);
        let departure = left
            .map(|at| (TAKEN_UP_REASON.to_string(), at))
            .or(github_moved);
        if facts.state != "live" {
            let recent = ended.and_then(|at| at.checked_add_signed(chrono::Duration::days(1)));
            // Keep subsequent transitions even while Done takes precedence.
            // Extra age/settlement wakes are conservative; all clocks use the
            // same captured departures and dismissal as grouping above.
            let retained = [
                ended.or_else(|| time(Some(facts.started_at))),
                taken_up,
                settled_at,
                recorded_dismissal.and_then(|row| time(Some(&row.at))),
                resolved_at,
            ]
            .into_iter()
            .flatten()
            .filter_map(|at| at.checked_add_signed(chrono::Duration::days(days)));
            let next = recent
                .into_iter()
                .chain(retained)
                // Grouping includes the cutoff; wake strictly after it.
                .filter_map(|at| at.checked_add_signed(chrono::Duration::nanoseconds(1)))
                .map(|at| at.with_timezone(&Local))
                .filter(|at| *at > now)
                .min();
            evaluation.next_change = evaluation.next_change.into_iter().chain(next).min();
        }
        if let Some(row) = command.selector() {
            for &url in &urls {
                if github.is_frozen(&row, url) {
                    continue;
                }
                if group == Group::History && github.has_answer(url) {
                    evaluation.refresh.freezes.push(Freeze {
                        row: row.clone(),
                        url: url.to_owned(),
                    });
                } else {
                    let triage_since = (captured.problem == Some(NeedsYouKind::Triage)
                        && records.iter().any(|record| {
                            record.issue.as_ref().is_some_and(|issue| issue.url == url)
                        }))
                    .then_some(ended)
                    .flatten();
                    subjects
                        .entry(url.to_owned())
                        .and_modify(|since: &mut Option<DateTime<FixedOffset>>| {
                            *since = (*since).max(triage_since);
                        })
                        .or_insert(triage_since);
                }
            }
            if let Observation::Record(command) = command
                && command.record.kind == "Security run"
                && command.record.security_findings_waiting
                && (group != Group::History || github.findings(&row).is_none())
            {
                evaluation.refresh.findings.push(FindingSubject {
                    row,
                    repository: command.record.repository.clone(),
                    urls: command
                        .record
                        .security_findings
                        .iter()
                        .map(|finding| finding.url.clone())
                        .collect(),
                });
            }
        }
        // Dismissed failures collapse together, separately from waiting ones.
        let failed = needs_you_kind == Some(NeedsYouKind::Failed)
            || dismissal
                .as_ref()
                .is_some_and(|row| row.kind == NeedsYouKind::Failed.title());
        evaluation.rows.push(Row {
            order: (
                group,
                needs_you_kind.map(NeedsYouKind::priority),
                Reverse(time(facts.ended_at.or(Some(facts.started_at)))),
            ),
            failure: facts
                .cause
                .filter(|cause| failed && !cause.is_empty())
                .map(|cause| (cause, dismissal.is_some())),
            item: InboxItem {
                command,
                group,
                needs_you_kind,
                landing,
                stopping,
                count: 1,
                collapsed: Vec::new(),
                departure: departure.map(|(reason, at)| Departure {
                    left_from: from.expect("departed rows waited").to_string(),
                    left_reason: reason,
                    left_at: at.to_rfc3339(),
                }),
                github_state: state.map(|state| state.state.to_string()),
                github_at: state.and_then(event_time).map(str::to_string),
                github: command
                    .selector()
                    .and_then(|row| github.row_answer(&row, urls)),
                dismissal,
            },
        });
    }
    evaluation.refresh.subjects = subjects
        .into_iter()
        .map(|(url, triage_since)| Subject { url, triage_since })
        .collect();
    evaluation
}

/// The row's own pull request URL, if its observation has a record with
/// one: a dispatched Ticket run's own, not its Pass's.
fn record_pull_request<'a>(observation: &Observation<'a>) -> Option<&'a str> {
    match observation {
        Observation::Record(command) => command.record.pull_request.as_deref(),
        Observation::History(_) => None,
    }
}

fn github_urls<'a>(
    observation: Observation<'a>,
    problem: Option<NeedsYouKind>,
    live_problem: Option<NeedsYouKind>,
    records: &[&'a crate::logs::command_record::CommandRecord],
) -> Vec<&'a str> {
    let mut urls = Vec::new();
    let problem = problem.is_some() || live_problem.is_some();
    if observation.facts().ready || problem {
        for record in records {
            if let Some(url) = &record.pull_request {
                urls.push(url.as_str());
            }
            if problem && let Some(issue) = &record.issue {
                urls.push(issue.url.as_str());
            }
        }
    }
    urls
}

fn github_known(
    observation: Observation<'_>,
    records: &[&crate::logs::command_record::CommandRecord],
    github: &Kept,
) -> bool {
    let Some(row) = observation.selector() else {
        return false;
    };
    records.iter().any(|record| {
        record
            .issue
            .as_ref()
            .is_some_and(|issue| github.issue(&row, &issue.url).is_some())
            || record
                .pull_request
                .as_ref()
                .is_some_and(|url| github.pull_request(&row, url).is_some())
    })
}

/// What GitHub last showed for the row's own pull request, if anything.
fn github_state<'a>(
    observation: &Observation<'a>,
    github: &'a Kept,
) -> Option<&'a PullRequestState> {
    github.pull_request(&observation.selector()?, record_pull_request(observation)?)
}

/// The pull request's merge or close time, as GitHub wrote it.
fn event_time(state: &PullRequestState) -> Option<&str> {
    state.merged_at.as_deref().or(state.closed_at.as_deref())
}

/// When an ended row stopped waiting through GitHub, with why it left.
/// A problem's issue closing or pull request merging settles it; Triage
/// also leaves on label removal. Ready rows also leave on an unmerged close.
fn github_left(
    captured: &Captured<'_>,
    records: &[&crate::logs::command_record::CommandRecord],
    github: &Kept,
    ended: Option<DateTime<FixedOffset>>,
) -> Option<(String, DateTime<FixedOffset>)> {
    let observation = captured.command;
    if observation.facts().state == "live" {
        return None;
    }
    let settled_work = github_work_left(captured, records, github, ended);
    let finding_row = captured.problem == Some(NeedsYouKind::Finding)
        || (captured.problem == Some(NeedsYouKind::Failed)
            && matches!(observation, Observation::Record(command) if command.record.security_findings_waiting
                && (settled_work.is_some() || captured.dismissal.as_ref()
                    .is_some_and(|row| row.covers(NeedsYouKind::Failed.title())
                        && !row.covers(NeedsYouKind::Finding.title())))));
    if finding_row
        && let Some(at) = observation
            .selector()
            .and_then(|row| github.findings(&row))
            .and_then(|facts| time(facts.triaged_at.as_deref()))
    {
        return Some(("findings triaged".to_string(), at));
    }
    settled_work
}

/// Settlement of the Command's own or dispatched work, independently of
/// any other findings that its Security audit left waiting.
fn github_work_left(
    captured: &Captured<'_>,
    records: &[&crate::logs::command_record::CommandRecord],
    github: &Kept,
    ended: Option<DateTime<FixedOffset>>,
) -> Option<(String, DateTime<FixedOffset>)> {
    let observation = captured.command;
    if captured.problem.is_some() {
        let row = observation.selector()?;
        let departures = records
            .iter()
            .map(|record| {
                let issue = record
                    .issue
                    .as_ref()
                    .and_then(|issue| github.issue(&row, &issue.url));
                let (reason, at) = if let Some(issue) = issue
                    && issue.closed
                {
                    (ISSUE_CLOSED_REASON, issue.closed_at.as_deref())
                } else if captured.problem == Some(NeedsYouKind::Triage)
                    && let Some(issue) = issue
                    && !issue.needs_triage
                {
                    // Absence before this Stop says nothing about whether
                    // its needs-triage label has since been removed.
                    let at = time(issue.triaged_at.as_deref())?;
                    if ended.is_some_and(|ended| at < ended) {
                        return None;
                    }
                    ("needs-triage removed", issue.triaged_at.as_deref())
                } else {
                    let pull = github.pull_request(&row, record.pull_request.as_deref()?)?;
                    if pull.state != PrState::Merged {
                        return None;
                    }
                    (MERGED_REASON, event_time(pull))
                };
                time(at)
                    .or(ended)
                    .or_else(|| time(Some(observation.facts().started_at)))
                    .map(|at| (reason.to_string(), at))
            })
            .collect::<Option<Vec<_>>>()?;
        return departures.into_iter().max_by_key(|(_, at)| *at);
    }
    if !observation.facts().ready {
        return None;
    }
    let state = github_state(&observation, github)?;
    let (reason, at) = match state.state {
        PrState::Merged => (MERGED_REASON.to_string(), event_time(state)),
        PrState::Closed => (CLOSED_REASON.to_string(), state.closed_at.as_deref()),
        PrState::Open => return None,
    };
    let at = at
        .and_then(|at| time(Some(at)))
        .or(ended)
        .or_else(|| time(Some(observation.facts().started_at)));
    at.map(|at| (reason, at))
}

/// Outer Command trees with live work anywhere in them, whatever their group:
/// a Spec run's Tickets, and the run a Pass dispatched, count within it.
pub(super) fn active_runs(entries: &[Entry]) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Record(command) if active(command)))
        .count()
}

fn active(command: &Command) -> bool {
    command.state == "live"
        || command
            .dispatched
            .iter()
            .chain(&command.children)
            .any(active)
}

fn live_problem(command: &Command) -> Option<NeedsYouKind> {
    let own = match command.health {
        Some(Health::Silent { .. }) => Some(NeedsYouKind::Silent),
        Some(Health::Slow { .. }) => Some(NeedsYouKind::Slow),
        _ => None,
    };
    own.into_iter()
        .chain(
            command
                .dispatched
                .iter()
                .chain(&command.children)
                .filter_map(live_problem),
        )
        .min()
}

fn is_landing(command: &Command) -> bool {
    (command.state == "live"
        && matches!(command.record.kind.as_str(), "Run" | "Spec run")
        && command
            .stage
            .as_ref()
            .is_some_and(|stage| matches!(stage.name.as_str(), "CI watch" | "Self-merge")))
        || command
            .dispatched
            .iter()
            .chain(&command.children)
            .any(is_landing)
}

/// Every start on the machine per issue, oldest first.
fn all_starts(
    starts: &[BTreeMap<(String, u64), DateTime<FixedOffset>>],
) -> BTreeMap<(String, u64), Vec<DateTime<FixedOffset>>> {
    let mut all: BTreeMap<(String, u64), Vec<DateTime<FixedOffset>>> = BTreeMap::new();
    for starts in starts {
        for (issue, start) in starts {
            all.entry(issue.clone()).or_default().push(*start);
        }
    }
    for times in all.values_mut() {
        times.sort();
    }
    all
}

/// The earliest later start after this row began waiting. An overlapping
/// Command cannot take up its ended work or hide a subsequent start that can.
fn superseded_at(
    starts: &BTreeMap<(String, u64), DateTime<FixedOffset>>,
    all: &BTreeMap<(String, u64), Vec<DateTime<FixedOffset>>>,
    ended: Option<DateTime<FixedOffset>>,
) -> Option<DateTime<FixedOffset>> {
    starts
        .iter()
        .filter_map(|(issue, start)| {
            all.get(issue).and_then(|times| {
                times
                    .iter()
                    .find(|time| **time > *start && ended.is_none_or(|end| **time >= end))
                    .copied()
            })
        })
        .min()
}

fn issue_starts(entry: Observation<'_>) -> BTreeMap<(String, u64), DateTime<FixedOffset>> {
    let mut starts = BTreeMap::new();
    fn collect(command: &Command, starts: &mut BTreeMap<(String, u64), DateTime<FixedOffset>>) {
        let record = &command.record;
        if let Some(issue) = &record.issue
            && let Some(start) = time(Some(&record.started_at))
        {
            starts
                .entry((record.repository.to_ascii_lowercase(), issue.number))
                .and_modify(|latest| *latest = (*latest).max(start))
                .or_insert(start);
        }
        for child in command.dispatched.iter().chain(&command.children) {
            collect(child, starts);
        }
    }
    match entry {
        Observation::Record(command) => collect(command, &mut starts),
        Observation::History(command) => {
            if let Some(issue) = &command.issue
                && let Some(start) = time(Some(&command.started_at))
            {
                starts.insert(
                    (command.repository.to_ascii_lowercase(), issue.number),
                    start,
                );
            }
        }
    }
    starts
}

fn time(value: Option<&str>) -> Option<DateTime<FixedOffset>> {
    value.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
}
