//! The machine's Inbox, computed from local observations for every reader.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, Local};
use serde::Serialize;

use super::{Command, Entry, Health, Observation};
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
}

impl InboxItem<'_> {
    pub(in crate::logs) fn cause(&self) -> Option<&str> {
        self.command.facts().cause
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
) -> Option<DateTime<Local>> {
    observations(entries)
        .filter_map(|observation| {
            let facts = observation.facts();
            if facts.state == "live" || facts.ready {
                return None;
            }
            let ended = time(facts.ended_at);
            let recent = ended.and_then(|at| at.checked_add_signed(chrono::Duration::days(1)));
            let retained = ended
                .or_else(|| time(Some(facts.started_at)))
                .and_then(|at| at.checked_add_signed(chrono::Duration::days(days)));
            [recent, retained]
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

pub(super) fn list(
    entries: &[Entry],
    now: DateTime<Local>,
    all: bool,
    days: i64,
) -> Vec<InboxItem<'_>> {
    // An accepted duration can exceed chrono's calendar. In that case every
    // representable timestamp is inside the requested retention period.
    let cutoff = now.checked_sub_signed(chrono::Duration::days(days));
    let observations: Vec<_> = observations(entries).collect();
    let starts: Vec<_> = observations.iter().copied().map(issue_starts).collect();
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
        let needs_you_kind = live_problem.or_else(|| {
            problem.filter(|_| {
                ended
                    .or_else(|| time(Some(facts.started_at)))
                    .is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff))
                    && !starts.iter().any(|(issue, start)| {
                        latest.get(issue).is_some_and(|latest| latest > start)
                    })
            })
        });
        let group = if needs_you_kind.is_some() {
            Group::NeedsYou
        } else if facts.ready {
            Group::ReadyForReview
        } else if landing {
            Group::Landing
        } else if facts.state == "live" {
            Group::Working
        } else if ended.is_some_and(|time| time >= now - chrono::Duration::days(1)) {
            Group::Done
        } else if ended.is_some_and(|time| cutoff.is_none_or(|cutoff| time >= cutoff)) {
            Group::Resolved
        } else {
            Group::History
        };
        if all || group != Group::History {
            items.push(InboxItem {
                command,
                group,
                needs_you_kind,
                landing,
                stopping,
                count: 1,
                collapsed: Vec::new(),
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
