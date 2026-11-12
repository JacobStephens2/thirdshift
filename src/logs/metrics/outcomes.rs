//! Outcome stacks per Command kind, kept apart so no kind's rate hides another.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::super::reading::{Command, Entry, Observation, Snapshot};
use super::scope::{Filters, Span, Window};

/// Commands that do work. A Pickup run only selects and dispatches one.
const WORK: [&str; 8] = [
    "Run",
    "Spec run",
    "Base fix",
    "Review run",
    "Fix run",
    "Triage run",
    "Architect run",
    "Security run",
];

/// How a Command stands: one of four ended stacks, or still live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Class {
    Succeeded,
    Failed,
    Paused,
    Stopped,
    Live,
}

impl Class {
    pub(crate) const ENDED: [Self; 4] =
        [Self::Succeeded, Self::Failed, Self::Paused, Self::Stopped];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Succeeded => "Succeeded",
            Self::Failed => "Failed",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Live => "Live",
        }
    }

    pub(crate) fn css(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Live => "live",
        }
    }
}

/// The class and exact sub-outcome of one Command, or `None` when it is out
/// of every denominator. `stopped` says a Stop was asked of it or its owner.
/// A Pickup run that dispatched work, or a skipped Pass: neither did work of
/// its own, so no trend counts it. A Pickup run's dispatch is counted instead.
pub(super) fn idle_or_pass_through(command: &Command) -> bool {
    command.record.outcome.as_deref() == Some("skipped")
        || (command.record.kind == "Pickup run"
            && !(command.dispatched.is_empty() && command.children.is_empty()))
}

fn classify(command: &Command, stopped: bool) -> Option<(Class, String)> {
    let kind = command.record.kind.as_str();
    let pickup = kind == "Pickup run";
    if idle_or_pass_through(command) {
        return None;
    }
    if !pickup && !WORK.contains(&kind) {
        return None;
    }
    let outcome = command.record.outcome.as_deref();
    let class = match (command.state, outcome) {
        ("live", _) => Class::Live,
        ("died", _) => Class::Failed,
        (_, None) => return None,
        (_, Some("paused")) => Class::Paused,
        (_, Some("interrupted")) if stopped => Class::Stopped,
        (_, Some(outcome))
            if outcome.contains("failed")
                || matches!(outcome, "review required" | "interrupted") =>
        {
            Class::Failed
        }
        (_, Some(_)) => Class::Succeeded,
    };
    if pickup && class != Class::Failed {
        return None;
    }
    let sub = match command.state {
        "live" | "died" => command.state,
        _ => outcome.unwrap_or_default(),
    };
    Some((class, sub.to_owned()))
}

pub(super) struct Counted {
    pub(super) bucket: NaiveDate,
    pub(super) account: Option<String>,
    pub(super) kind: &'static str,
    pub(super) class: Class,
    outcome: String,
    /// What `thirdshift show` and the Console's Command details select it by.
    pub(super) selector: String,
}

/// One counted Command, kept with its record for the signal groups.
pub(super) struct Fact<'a> {
    pub(super) counted: Counted,
    pub(super) command: &'a Command,
    /// A Stop was asked of this Command itself, not only of its owner.
    pub(super) stop_asked: bool,
}

/// Whether a Command is in the Succeeded stack. A Stop never makes one.
pub(super) fn succeeded(command: &Command) -> bool {
    classify(command, false).is_some_and(|(class, _)| class == Class::Succeeded)
}

/// Counts of one group. `n` is the rate's denominator: Succeeded and Failed.
/// `observed` Commands have ended or died; `missing` ones are live, with no
/// outcome yet. A Fleet machine's tally is read back from its own JSON.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Tally {
    pub(crate) runs: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) paused: usize,
    pub(crate) stopped: usize,
    pub(crate) live: usize,
    pub(crate) n: usize,
    pub(crate) observed: usize,
    pub(crate) missing: usize,
    pub(crate) success_rate: Option<f64>,
    pub(crate) sub_outcomes: BTreeMap<Class, BTreeMap<String, usize>>,
}

impl Tally {
    pub(super) fn add(&mut self, counted: &Counted) {
        self.runs += 1;
        *match counted.class {
            Class::Succeeded => &mut self.succeeded,
            Class::Failed => &mut self.failed,
            Class::Paused => &mut self.paused,
            Class::Stopped => &mut self.stopped,
            Class::Live => &mut self.live,
        } += 1;
        *self
            .sub_outcomes
            .entry(counted.class)
            .or_default()
            .entry(counted.outcome.clone())
            .or_default() += 1;
        self.settle();
    }

    /// Adds another machine's counts: each Command is counted where it ran.
    pub(super) fn merge(&mut self, other: &Self) {
        self.runs += other.runs;
        self.succeeded += other.succeeded;
        self.failed += other.failed;
        self.paused += other.paused;
        self.stopped += other.stopped;
        self.live += other.live;
        for (class, outcomes) in &other.sub_outcomes {
            for (outcome, count) in outcomes {
                *self
                    .sub_outcomes
                    .entry(*class)
                    .or_default()
                    .entry(outcome.clone())
                    .or_default() += count;
            }
        }
        self.settle();
    }

    fn settle(&mut self) {
        self.n = self.succeeded + self.failed;
        self.missing = self.live;
        self.observed = self.runs - self.live;
        self.success_rate = (self.n > 0).then(|| self.succeeded as f64 / self.n as f64);
    }

    pub(crate) fn count(&self, class: Class) -> usize {
        match class {
            Class::Succeeded => self.succeeded,
            Class::Failed => self.failed,
            Class::Paused => self.paused,
            Class::Stopped => self.stopped,
            Class::Live => self.live,
        }
    }

    /// `75% (3/4)`, or `none ended` with no Succeeded or Failed Command.
    pub(crate) fn rate(&self) -> String {
        match self.success_rate {
            Some(rate) => format!("{:.0}% ({}/{})", rate * 100.0, self.succeeded, self.n),
            None => "none ended".into(),
        }
    }

    /// The ended stacks as segments of a 100-wide bar, each with its exact
    /// sub-outcomes for hover.
    pub(crate) fn segments(&self) -> Vec<Segment> {
        let ended = self.observed.max(1) as f64;
        let mut x = 0.0;
        Class::ENDED
            .into_iter()
            .filter(|class| self.count(*class) > 0)
            .map(|class| {
                let width = 100.0 * self.count(class) as f64 / ended;
                let segment = Segment {
                    css: class.css(),
                    x: format!("{x:.2}"),
                    width: format!("{width:.2}"),
                    title: self.breakdown(class),
                };
                x += width;
                segment
            })
            .collect()
    }

    /// `Failed 3: died 1, failed 2`.
    pub(crate) fn breakdown(&self, class: Class) -> String {
        let outcomes = self
            .sub_outcomes
            .get(&class)
            .into_iter()
            .flatten()
            .map(|(outcome, count)| format!("{outcome} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{} {}: {outcomes}", class.name(), self.count(class))
    }

    pub(crate) fn summary(&self) -> String {
        Class::ENDED
            .into_iter()
            .chain([Class::Live])
            .map(|class| format!("{} {}", self.count(class), class.name().to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub(crate) struct Segment {
    pub(crate) css: &'static str,
    pub(crate) x: String,
    pub(crate) width: String,
    pub(crate) title: String,
}

/// One bucket's counts for one OS account and Command kind.
#[derive(Serialize)]
pub(crate) struct Bucket {
    pub(crate) bucket: NaiveDate,
    pub(crate) account: Option<String>,
    pub(crate) kind: &'static str,
    #[serde(flatten)]
    pub(crate) tally: Tally,
    /// The Failed Commands' selectors, for `thirdshift show` and History.
    pub(crate) failed_commands: Vec<String>,
}

/// The window against the one before it, per OS account and Command kind.
#[derive(Serialize)]
pub(crate) struct Comparison {
    pub(crate) current: Span,
    pub(crate) prior: Span,
    pub(crate) rows: Vec<Change>,
}

#[derive(Serialize)]
pub(crate) struct Change {
    pub(crate) account: Option<String>,
    pub(crate) kind: &'static str,
    pub(crate) current: Tally,
    pub(crate) prior: Tally,
    pub(crate) delta: Delta,
}

/// Current minus prior. The rate's change is missing unless both have one.
#[derive(Serialize)]
pub(crate) struct Delta {
    pub(crate) success_rate: Option<f64>,
    pub(crate) runs: i64,
    pub(crate) succeeded: i64,
    pub(crate) failed: i64,
    pub(crate) paused: i64,
    pub(crate) stopped: i64,
}

impl Delta {
    pub(super) fn of(current: &Tally, prior: &Tally) -> Self {
        let change = |current: usize, prior: usize| current as i64 - prior as i64;
        Self {
            success_rate: current
                .success_rate
                .zip(prior.success_rate)
                .map(|(current, prior)| current - prior),
            runs: change(current.runs, prior.runs),
            succeeded: change(current.succeeded, prior.succeeded),
            failed: change(current.failed, prior.failed),
            paused: change(current.paused, prior.paused),
            stopped: change(current.stopped, prior.stopped),
        }
    }

    /// `+12 pts`, or `missing` without a rate on both sides.
    pub(crate) fn rate(&self) -> String {
        match self.success_rate {
            Some(change) => format!("{:+.0} pts", change * 100.0),
            None => "missing".into(),
        }
    }
}

pub(super) struct Outcomes {
    pub(super) buckets: Vec<Bucket>,
    pub(super) comparison: Option<Comparison>,
}

/// Every counted Command the filters admit, in any bucket: the window and the
/// one before it are chosen by each reader.
pub(super) fn facts<'a>(
    snapshot: &'a Snapshot,
    window: &Window,
    filters: &Filters,
    serving: Option<&str>,
) -> Vec<Fact<'a>> {
    let mut facts = Vec::new();
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry {
            visit(command, false, window, filters, serving, &mut facts);
        }
    }
    facts
}

pub(super) fn read(facts: &[Fact<'_>], window: &Window) -> Outcomes {
    let counted: Vec<_> = facts.iter().map(|fact| &fact.counted).collect();
    let mut buckets = BTreeMap::<_, (Tally, Vec<String>)>::new();
    for item in counted.iter().filter(|item| window.shows(item.bucket)) {
        let (tally, failed) = buckets
            .entry((item.bucket, item.account.clone(), item.kind))
            .or_default();
        tally.add(item);
        if item.class == Class::Failed {
            failed.push(item.selector.clone());
        }
    }
    Outcomes {
        buckets: buckets
            .into_iter()
            .map(|((bucket, account, kind), (tally, failed))| Bucket {
                bucket,
                account,
                kind,
                tally,
                failed_commands: failed,
            })
            .collect(),
        comparison: window
            .current()
            .zip(window.prior)
            .map(|(current, prior)| compare(&counted, current, prior)),
    }
}

fn compare(counted: &[&Counted], current: Span, prior: Span) -> Comparison {
    let mut rows = BTreeMap::<_, (Tally, Tally)>::new();
    for item in counted {
        let key = (item.account.clone(), item.kind);
        if current.contains(item.bucket) {
            rows.entry(key).or_default().0.add(item);
        } else if prior.contains(item.bucket) {
            rows.entry(key).or_default().1.add(item);
        }
    }
    Comparison {
        current,
        prior,
        rows: rows
            .into_iter()
            .map(|((account, kind), (current, prior))| Change {
                account,
                kind,
                delta: Delta::of(&current, &prior),
                current,
                prior,
            })
            .collect(),
    }
}

fn visit<'a>(
    command: &'a Command,
    owner_stopped: bool,
    window: &Window,
    filters: &Filters,
    serving: Option<&str>,
    facts: &mut Vec<Fact<'a>>,
) {
    // A Stop of a Spec run stops its Ticket runs; they share its request, as a
    // dispatch shares its owner's record.
    let stop_asked =
        !owner_stopped && super::super::stop::requested(&command.record_path).is_some();
    let stopped = owner_stopped || stop_asked;
    for child in command.dispatched.iter().chain(&command.children) {
        visit(child, stopped, window, filters, serving, facts);
    }
    let account = command.account.as_deref().or(serving);
    if !filters.admit(command, account) {
        return;
    }
    let (Some(bucket), Some((class, outcome))) =
        (window.bucket(command), classify(command, stopped))
    else {
        return;
    };
    facts.push(Fact {
        counted: Counted {
            bucket,
            account: account.map(str::to_owned),
            kind: kind_name(&command.record.kind),
            class,
            outcome,
            selector: Observation::Record(command).selector().unwrap_or_default(),
        },
        command,
        stop_asked,
    });
}

/// The stored kinds an outcome row can have: the work Commands, then Pickup runs.
pub(crate) const COUNTED: [&str; 9] = [
    WORK[0],
    WORK[1],
    WORK[2],
    WORK[3],
    WORK[4],
    WORK[5],
    WORK[6],
    WORK[7],
    "Pickup run",
];

fn kind_name(stored: &str) -> &'static str {
    COUNTED
        .into_iter()
        .find(|kind| *kind == stored)
        .map(crate::logs::display_name)
        .expect("only work kinds and Pickup runs are counted")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(class: Class, outcome: &str) -> Counted {
        Counted {
            bucket: NaiveDate::default(),
            account: None,
            kind: "Ticket run",
            class,
            outcome: outcome.into(),
            selector: String::new(),
        }
    }

    #[test]
    fn live_runs_are_missing_outcomes_while_died_runs_are_failed() {
        let mut tally = Tally::default();
        for (class, outcome) in [
            (Class::Live, "live"),
            (Class::Failed, "died"),
            (Class::Succeeded, "merged"),
            (Class::Stopped, "interrupted"),
        ] {
            tally.add(&counted(class, outcome));
        }
        assert_eq!((tally.runs, tally.live, tally.failed), (4, 1, 1));
        assert_eq!((tally.n, tally.observed, tally.missing), (2, 3, 1));
        assert_eq!(tally.rate(), "50% (1/2)");
        assert_eq!(tally.breakdown(Class::Failed), "Failed 1: died 1");
        assert_eq!(
            tally.summary(),
            "1 succeeded, 1 failed, 0 paused, 1 stopped, 1 live"
        );
        // Bars stack only ended Commands, a third each.
        let bars: Vec<_> = tally
            .segments()
            .into_iter()
            .map(|segment| (segment.css, segment.x, segment.width))
            .collect();
        assert_eq!(
            bars,
            [
                ("succeeded", "0.00".into(), "33.33".into()),
                ("failed", "33.33".into(), "33.33".into()),
                ("stopped", "66.67".into(), "33.33".into()),
            ]
        );
    }

    #[test]
    fn deltas_need_a_rate_on_both_sides() {
        let mut current = Tally::default();
        current.add(&counted(Class::Succeeded, "merged"));
        let prior = Tally::default();
        let delta = Delta::of(&current, &prior);
        assert_eq!(
            (delta.rate(), delta.runs, delta.succeeded),
            ("missing".into(), 1, 1)
        );
        let mut prior = Tally::default();
        prior.add(&counted(Class::Failed, "failed"));
        prior.add(&counted(Class::Succeeded, "merged"));
        assert_eq!(Delta::of(&current, &prior).rate(), "+50 pts");
    }
}
