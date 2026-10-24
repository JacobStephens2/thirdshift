//! The machine's Inbox, computed from local observations for every reader.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Local};
use serde::Serialize;

use super::{Command, Entry, Health, Observation};
use crate::github::{PrState, PullRequestState};
use crate::logs::dismiss::{self, DismissalView, READY_KIND};
use crate::logs::github::{Kept, Subject};
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
    /// Who dismissed the row, when, from where and all kinds covered.
    /// `None` while the row waits or was never dismissed for its kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissal: Option<DismissalView>,
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

    pub(in crate::logs) fn github_urls(&self) -> Vec<&str> {
        github_urls(self.command)
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
    fn problem(self) -> Option<NeedsYouKind> {
        let facts = self.facts();
        if facts.state == "died" {
            Some(NeedsYouKind::Died)
        } else if matches!(self, Observation::Record(command) if facts.state == "ended" && !facts.ready && stop::requested(&command.record_path).is_some_and(|request| !request.requeue))
        {
            Some(NeedsYouKind::Triage)
        } else if facts
            .outcome
            .is_some_and(|outcome| outcome.contains("failed") || outcome == "review required")
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
    github: &Kept,
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
            // GitHub departures stay in Resolved for inbox.days from settlement.
            let github_moved = github_left(&observation, github, ended)
                .and_then(|(_, at)| at.checked_add_signed(chrono::Duration::days(days)));
            // A dismissed row stays in Resolved until inbox.days after the
            // dismissal, so its new boundary wakes open pages too.
            let dismissed = match observation {
                Observation::Record(command) => observation
                    .selector()
                    .and_then(|selector| dismiss::covering(&command.record_path, &selector))
                    .and_then(|row| time(Some(&row.at)))
                    .and_then(|at| at.checked_add_signed(chrono::Duration::days(days))),
                Observation::History(_) => None,
            };
            [recent, retained, taken_up, github_moved, dismissed]
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

/// Issues/pull requests for Needs you and pull requests for Ready for
/// review, including departures still in view. History keeps its last facts;
/// a row not yet read gets one lookup before local age alone can move it there.
pub(super) fn github_subjects(
    entries: &[Entry],
    now: DateTime<Local>,
    days: i64,
    github: &Kept,
) -> Vec<Subject> {
    let mut urls = BTreeMap::new();
    for item in list(entries, now, true, days, github) {
        for item in std::iter::once(&item).chain(&item.collapsed) {
            for url in item.github_urls() {
                if let Some(row) = item.command.selector()
                    && !github.is_frozen(&row, url)
                    && (item.group != Group::History || !github.has_answer(url))
                {
                    let triage_since = (item.command.problem() == Some(NeedsYouKind::Triage)
                        && github_records(item.command).iter().any(|record| {
                            record.issue.as_ref().is_some_and(|issue| issue.url == url)
                        }))
                    .then(|| time(item.command.facts().ended_at))
                    .flatten();
                    urls.entry(url.to_owned())
                        .and_modify(|since: &mut Option<DateTime<FixedOffset>>| {
                            *since = (*since).max(triage_since)
                        })
                        .or_insert(triage_since);
                }
            }
        }
    }
    urls.into_iter()
        .map(|(url, triage_since)| Subject { url, triage_since })
        .collect()
}

pub(super) fn list<'a>(
    entries: &'a [Entry],
    now: DateTime<Local>,
    all: bool,
    days: i64,
    github: &Kept,
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
        let recorded_dismissal = match command {
            Observation::Record(record) => command
                .selector()
                .and_then(|selector| dismiss::covering(&record.record_path, &selector)),
            Observation::History(_) => None,
        };
        let findings_waiting = matches!(command, Observation::Record(command) if command.record.security_findings_waiting);
        let github_settlement = github_left(&command, github, ended);
        // Settling failed work removes that wait, while another finding
        // from the same audit can still need the Day shift.
        let finding_wait = github_settlement.is_some() && findings_waiting;
        let mut problem = if finding_wait {
            Some(NeedsYouKind::Finding)
        } else {
            command.problem()
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
                    && github_known(command, github)));
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
        let group = if needs_you_kind.is_some() {
            Group::NeedsYou
        } else if facts.ready && held && github_moved.is_none() && !dismissed {
            Group::ReadyForReview
        } else if landing {
            Group::Landing
        } else if facts.state == "live" {
            Group::Working
        } else if ended.is_some_and(|time| time >= now - chrono::Duration::days(1)) {
            Group::Done
        } else if (github_moved.is_none()
            && ended.is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff)))
            || left.is_some_and(|taken| {
                cutoff.is_none_or(|cutoff| taken.with_timezone(&Local) >= cutoff)
            })
            || github_moved.as_ref().is_some_and(|(_, at)| {
                cutoff.is_none_or(|cutoff| at.with_timezone(&Local) >= cutoff)
            })
            || dismissed_at
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
        if all || group != Group::History {
            items.push(InboxItem {
                command,
                group,
                needs_you_kind,
                landing,
                stopping,
                count: 1,
                collapsed: Vec::new(),
                left_from: departure
                    .as_ref()
                    .map(|_| from.expect("departed rows waited").to_string()),
                left_reason: departure.as_ref().map(|(reason, _)| reason.clone()),
                left_at: departure.as_ref().map(|(_, at)| at.to_rfc3339()),
                github_state: state.map(|state| state.state.to_string()),
                github_at: state.and_then(event_time).map(str::to_string),
                dismissal,
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
        // A dismissed Failed row keeps the row it left: dismissed failures
        // collapse with each other, never with failures still waiting.
        let failed = item.needs_you_kind == Some(NeedsYouKind::Failed)
            || item
                .dismissal
                .as_ref()
                .is_some_and(|dismissal| dismissal.kind == NeedsYouKind::Failed.title());
        if failed && let Some(cause) = item.command.facts().cause.filter(|cause| !cause.is_empty())
        {
            // A later failure with a dismissed cause starts a row of one:
            // dismissed rows collapse only with each other.
            let key = (cause, item.dismissal.is_some());
            if let Some(&index) = failures.get(&key) {
                let first: &mut InboxItem<'_> = &mut collapsed[index];
                first.count += 1;
                first.collapsed.push(item);
                continue;
            }
            failures.insert(key, collapsed.len());
        }
        collapsed.push(item);
    }
    collapsed
}

/// The row's own pull request URL, if its observation has a record with
/// one: a dispatched Ticket run's own, not its Pass's.
fn record_pull_request<'a>(observation: &Observation<'a>) -> Option<&'a str> {
    match observation {
        Observation::Record(command) => command.record.pull_request.as_deref(),
        Observation::History(_) => None,
    }
}

fn github_urls(observation: Observation<'_>) -> Vec<&str> {
    let mut urls = Vec::new();
    let problem = observation.problem().is_some()
        || matches!(observation, Observation::Record(command) if live_problem(command).is_some());
    if observation.facts().ready || problem {
        for record in github_records(observation) {
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

/// A failed Pass is answered by its failed dispatched work, not by a
/// different fix that succeeded. Undispatched failures have no such subject.
fn github_records(
    observation: Observation<'_>,
) -> Vec<&crate::logs::command_record::CommandRecord> {
    // A successful fix speaks for that fix, not for other findings from
    // its audit that still need the Day shift.
    if observation.problem() == Some(NeedsYouKind::Finding) {
        return Vec::new();
    }
    let Observation::Record(command) = observation else {
        return Vec::new();
    };
    if observation.problem().is_some()
        && matches!(
            command.record.kind.as_str(),
            "Architect run" | "Security run"
        )
        && !command.dispatched.is_empty()
    {
        let failed: Vec<_> = command
            .dispatched
            .iter()
            .filter(|run| Observation::Record(run).problem().is_some())
            .map(|run| &run.record)
            .collect();
        if !failed.is_empty() {
            return failed;
        }
        return command.dispatched.iter().map(|run| &run.record).collect();
    }
    vec![&command.record]
}

fn github_known(observation: Observation<'_>, github: &Kept) -> bool {
    let Some(row) = observation.selector() else {
        return false;
    };
    github_records(observation).iter().any(|record| {
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
    observation: &Observation<'_>,
    github: &Kept,
    ended: Option<DateTime<FixedOffset>>,
) -> Option<(String, DateTime<FixedOffset>)> {
    if observation.facts().state == "live" {
        return None;
    }
    if observation.problem().is_some() {
        let row = observation.selector()?;
        let departures = github_records(*observation)
            .into_iter()
            .map(|record| {
                let issue = record
                    .issue
                    .as_ref()
                    .and_then(|issue| github.issue(&row, &issue.url));
                let (reason, at) = if let Some(issue) = issue
                    && issue.closed
                {
                    (ISSUE_CLOSED_REASON, issue.closed_at.as_deref())
                } else if observation.problem() == Some(NeedsYouKind::Triage)
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
    let state = github_state(observation, github)?;
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
