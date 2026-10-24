//! The machine's Inbox, computed from local observations for every reader.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Local};
use serde::Serialize;

use super::{Command, Entry, Health, Observation};
use crate::github::{PrState, PullRequestState};
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
/// Why a row left Ready for review through GitHub: its pull request merged.
pub(in crate::logs) const MERGED_REASON: &str = "pull request merged";
/// Why a row left Ready for review through GitHub: its pull request closed.
pub(in crate::logs) const CLOSED_REASON: &str = "pull request closed";

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
    /// Which waiting group the row left, when it did so through the
    /// later-Command exit. `None` for rows that never waited or that aged out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_reason: Option<String>,
    /// The later Command's start, RFC 3339: when the row stopped waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_at: Option<String>,
    /// What GitHub last showed for the row's pull request: `open`,
    /// `merged` or `closed`. `None` where GitHub speaks for no row: one
    /// with no pull request, one not read yet, or one GitHub can't resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_state: Option<String>,
    /// The pull request's merge or close time, RFC 3339, for a merged or
    /// closed row: when the row stopped waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_at: Option<String>,
}

impl InboxItem<'_> {
    pub(in crate::logs) fn cause(&self) -> Option<&str> {
        self.command.facts().cause
    }

    /// The line saying why and when the row left its waiting group, for
    /// `status` text and the Console row. `None` while it still waits or
    /// never waited.
    pub(in crate::logs) fn left_line(&self) -> Option<String> {
        match (&self.left_from, &self.left_reason, &self.left_at) {
            (Some(from), Some(reason), Some(at)) if reason == TAKEN_UP_REASON => {
                Some(format!("left {from}: {reason} started at {at}"))
            }
            (Some(from), Some(reason), Some(at)) => Some(format!("left {from}: {reason} at {at}")),
            _ => None,
        }
    }

    /// The row's own pull request, judged by GitHub: a dispatched Ticket
    /// run's own, not its Pass's. `None` for rows with no record.
    pub(in crate::logs) fn pull_request(&self) -> Option<&str> {
        match self.command {
            Observation::Record(command) => command.record.pull_request.as_deref(),
            Observation::History(_) => None,
        }
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

fn observations(entries: &[Entry]) -> impl Iterator<Item = Observation<'_>> {
    entries.iter().flat_map(|entry| {
        let (outer, ready): (_, Vec<_>) = match entry {
            Entry::Record(command) => (
                Observation::Record(command),
                command
                    .dispatched
                    .iter()
                    .filter(|run| Observation::Record(run).facts().ready)
                    .map(Observation::Record)
                    .collect(),
            ),
            Entry::History(command) => (Observation::History(command), Vec::new()),
        };
        std::iter::once(outer).chain(ready)
    })
}

pub(super) fn next_change(
    entries: &[Entry],
    now: DateTime<Local>,
    days: i64,
    github: &BTreeMap<String, Option<PullRequestState>>,
) -> Option<DateTime<Local>> {
    let observations: Vec<_> = observations(entries).collect();
    let starts: Vec<_> = observations.iter().copied().map(issue_starts).collect();
    let starts_by_issue = all_starts(&starts);
    observations
        .into_iter()
        .zip(&starts)
        .filter_map(|(observation, starts)| {
            let facts = observation.facts();
            if facts.state == "live" {
                return None;
            }
            let ended = time(facts.ended_at);
            let recent = ended.and_then(|at| at.checked_add_signed(chrono::Duration::days(1)));
            let retained = ended
                .or_else(|| time(Some(facts.started_at)))
                .and_then(|at| at.checked_add_signed(chrono::Duration::days(days)));
            // A row taken up by a later Command stays in Resolved until
            // inbox.days after that Command started, so its new boundary
            // wakes open pages too.
            let taken_up = superseded_at(starts, &starts_by_issue)
                .and_then(|at| at.checked_add_signed(chrono::Duration::days(days)));
            // A row GitHub moved out of Ready for review stays in Resolved
            // until inbox.days after its pull request merged or closed.
            let github_moved = github_left(&observation, github, ended)
                .and_then(|(_, at)| at.checked_add_signed(chrono::Duration::days(days)));
            [recent, retained, taken_up, github_moved]
                .into_iter()
                .flatten()
                // Grouping includes the exact cutoff; cross it before waking.
                .filter_map(|at| at.checked_add_signed(chrono::Duration::nanoseconds(1)))
                .map(|at| at.with_timezone(&Local))
                .filter(|at| *at > now)
                .min()
        })
        .min()
}

/// Every ready row's own pull request, deduplicated: a dispatched Ticket
/// run's own, not its Pass's. These are the pull requests the next GitHub
/// read looks up: every row in Ready for review, and every row that left
/// it, while still in view.
pub(super) fn ready_pull_requests(entries: &[Entry]) -> Vec<String> {
    let mut urls = BTreeMap::new();
    for observation in observations(entries) {
        if observation.facts().ready
            && let Observation::Record(command) = observation
            && let Some(url) = &command.record.pull_request
        {
            urls.insert(url.clone(), ());
        }
    }
    urls.into_keys().collect()
}

pub(super) fn list<'a>(
    entries: &'a [Entry],
    now: DateTime<Local>,
    all: bool,
    days: i64,
    github: &BTreeMap<String, Option<PullRequestState>>,
) -> Vec<InboxItem<'a>> {
    // An accepted duration can exceed chrono's calendar. In that case every
    // representable timestamp is inside the requested retention period.
    let cutoff = now.checked_sub_signed(chrono::Duration::days(days));
    let observations: Vec<_> = observations(entries).collect();
    let starts: Vec<_> = observations.iter().copied().map(issue_starts).collect();
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
    let mut items = Vec::new();
    for (command, starts) in observations.into_iter().zip(&starts) {
        let facts = command.facts();
        let ended = time(facts.ended_at);
        let problem = if facts.state == "died" {
            Some(NeedsYouKind::Died)
        } else if matches!(command, Observation::Record(command) if facts.state == "ended" && !facts.ready && stop::requested(&command.record_path).is_some_and(|request| !request.requeue))
        {
            Some(NeedsYouKind::Triage)
        } else if facts
            .outcome
            .is_some_and(|outcome| outcome.contains("failed") || outcome == "review required")
        {
            Some(NeedsYouKind::Failed)
        } else if matches!(command, Observation::Record(command) if command.record.security_findings_waiting)
        {
            Some(NeedsYouKind::Finding)
        } else {
            None
        };
        let live_problem = match command {
            Observation::Record(command) if facts.state == "live" => live_problem(command),
            _ => None,
        };
        let landing = matches!(command, Observation::Record(command) if is_landing(command));
        let stopping = matches!(command, Observation::Record(command) if facts.state == "live" && stop::requested(&command.record_path).is_some());
        // When this row stopped waiting through the later-Command exit:
        // the earliest later start on any of its issues. Only rows that
        // waited (ended problems and ready Commands) leave through it, so
        // a Command that never waited keeps going by when it ended, and a
        // row that ages out keeps going straight to History.
        let waited = problem.is_some() || facts.ready;
        let taken_up = if waited {
            superseded_at(starts, &starts_by_issue).filter(|taken| {
                // The exit only ends a wait that had already started: the
                // later Command must have started at or after this row
                // ended. A row superseded while still live never waited.
                ended.is_none_or(|end| *taken >= end)
            })
        } else {
            None
        };
        // The Inbox holds an ended problem or ready Command until a later
        // Command on the same issue starts here, or it outlives the period.
        // A ready row whose pull request GitHub still shows as open stays,
        // however old it is: age moves only rows GitHub can't speak for. A
        // row a later Command already took up stays taken up.
        let github_open = taken_up.is_none()
            && facts.ready
            && github_state(&command, github).is_some_and(|state| state.state == PrState::Open);
        let held = (ended
            .or_else(|| time(Some(facts.started_at)))
            .is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff))
            && !starts
                .iter()
                .any(|(issue, start)| latest.get(issue).is_some_and(|latest| latest > start)))
            || github_open;
        let needs_you_kind = live_problem.or_else(|| problem.filter(|_| held));
        let from = if problem.is_some() {
            Some("needs you")
        } else if facts.ready {
            Some("ready for review")
        } else {
            None
        };
        let left = taken_up.filter(|_| from.is_some());
        // When this row stopped waiting through GitHub: a ready row whose
        // pull request merged or closed. The later-Command exit wins where
        // both apply, and a reopened pull request brings back only a row
        // still in Done or Resolved: History rows are not read again.
        let moved = left
            .is_none()
            .then(|| github_left(&command, github, ended))
            .flatten()
            .filter(|_| from.is_some());
        let group = if needs_you_kind.is_some() {
            Group::NeedsYou
        } else if facts.ready && held && moved.is_none() {
            Group::ReadyForReview
        } else if landing {
            Group::Landing
        } else if facts.state == "live" {
            Group::Working
        } else if ended.is_some_and(|time| time >= now - chrono::Duration::days(1)) {
            Group::Done
        } else if ended.is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff))
            || left.is_some_and(|taken| {
                cutoff.is_none_or(|cutoff| taken.with_timezone(&Local) >= cutoff)
            })
            || moved.as_ref().is_some_and(|(_, at)| {
                cutoff.is_none_or(|cutoff| at.with_timezone(&Local) >= cutoff)
            })
        {
            Group::Resolved
        } else {
            Group::History
        };
        // A row GitHub finished weeks ago goes straight to History when
        // first read, instead of filling Resolved at once.
        let state = github_state(&command, github);
        if all || group != Group::History {
            items.push(InboxItem {
                command,
                group,
                needs_you_kind,
                landing,
                stopping,
                count: 1,
                collapsed: Vec::new(),
                left_from: left
                    .map(|_| from.expect("left rows waited").to_string())
                    .or(moved
                        .as_ref()
                        .map(|_| from.expect("moved rows waited").to_string())),
                left_reason: left
                    .map(|_| TAKEN_UP_REASON.to_string())
                    .or(moved.as_ref().map(|(reason, _)| reason.clone())),
                left_at: left
                    .map(|taken| taken.to_rfc3339())
                    .or(moved.as_ref().map(|(_, at)| at.to_rfc3339())),
                github_state: state.map(|state| state.state.to_string()),
                github_at: state.and_then(|state| {
                    let at = state.merged_at.as_deref().or(state.closed_at.as_deref());
                    at.map(str::to_string)
                }),
            });
        }
    }
    items.sort_by_key(|item| {
        let facts = item.command.facts();
        (
            item.group,
            item.needs_you_kind.map(NeedsYouKind::priority),
            Reverse(time(facts.ended_at.or(Some(facts.started_at)))),
        )
    });
    let mut collapsed: Vec<InboxItem<'_>> = Vec::new();
    let mut failures = BTreeMap::new();
    for item in items {
        if item.needs_you_kind == Some(NeedsYouKind::Failed)
            && let Some(cause) = item.command.facts().cause.filter(|cause| !cause.is_empty())
        {
            if let Some(&index) = failures.get(cause) {
                let first: &mut InboxItem<'_> = &mut collapsed[index];
                first.count += 1;
                first.collapsed.push(item);
                continue;
            }
            failures.insert(cause, collapsed.len());
        }
        collapsed.push(item);
    }
    collapsed
}

/// What GitHub last showed for the row's own pull request, if anything: a
/// dispatched Ticket run's own, not its Pass's.
fn github_state<'a>(
    observation: &Observation<'a>,
    github: &'a BTreeMap<String, Option<PullRequestState>>,
) -> Option<&'a PullRequestState> {
    match observation {
        Observation::Record(command) => command
            .record
            .pull_request
            .as_deref()
            .and_then(|url| github.get(url).and_then(Option::as_ref)),
        Observation::History(_) => None,
    }
}

/// When the row stopped waiting through GitHub: its pull request's merge
/// or close time, with why it left. Only ready rows move this way; any
/// other ready row's age no longer moves it, and a row with no pull
/// request, no read yet, or an unresolvable pull request still ages out
/// by the usual rule.
fn github_left(
    observation: &Observation<'_>,
    github: &BTreeMap<String, Option<PullRequestState>>,
    ended: Option<DateTime<FixedOffset>>,
) -> Option<(String, DateTime<FixedOffset>)> {
    if !observation.facts().ready {
        return None;
    }
    let state = github_state(observation, github)?;
    let (reason, at) = match state.state {
        PrState::Merged => (
            MERGED_REASON.to_string(),
            state.merged_at.as_deref().or(state.closed_at.as_deref()),
        ),
        PrState::Closed => (CLOSED_REASON.to_string(), state.closed_at.as_deref()),
        PrState::Open => return None,
    };
    let at = at
        .and_then(|at| time(Some(at)))
        .or(ended)
        .or_else(|| time(Some(observation.facts().started_at)));
    at.map(|at| (reason, at))
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

/// The earliest start on any of this row's issues after its own start:
/// when a later Command took it up. `None` when no later Command started.
fn superseded_at(
    starts: &BTreeMap<(String, u64), DateTime<FixedOffset>>,
    all: &BTreeMap<(String, u64), Vec<DateTime<FixedOffset>>>,
) -> Option<DateTime<FixedOffset>> {
    starts
        .iter()
        .filter_map(|(issue, start)| {
            all.get(issue)
                .and_then(|times| times.iter().find(|time| **time > *start).copied())
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
