//! The four paired signal groups: yield and kept pull-request survival,
//! post-merge quality and attention, survivorship-safe efficiency, and stage
//! and session friction. Each line keeps its own units and coverage, per
//! bucket for its sparkline and against the preceding window when bounded.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use super::super::command_record::StageName;
use super::super::reading::Command;
use super::super::run_metrics::Position;
use super::super::session_signals::Signals;
use super::bug_origins::ConfirmedCounts;
use super::outcomes::{Class, Fact};
use super::scope::{Filters, Window};
use super::trends::Signal;
use crate::github::{PrState, PullRequestState};
use crate::scoring::ScoreHistory;

/// What the groups read besides Command records, all of it local.
pub(super) struct Sources<'a> {
    pub(super) origins: &'a ConfirmedCounts,
    /// Kept pull-request facts by [`super::super::github::pull_request_key`].
    pub(super) kept: &'a BTreeMap<String, PullRequestState>,
    pub(super) scores: &'a ScoreHistory,
    pub(super) filters: &'a Filters,
}

#[derive(Serialize)]
pub(crate) struct Groups {
    #[serde(rename = "yield")]
    pub(crate) yield_: Yield,
    pub(crate) quality: Quality,
    pub(crate) efficiency: Group<Setup, Efficiency>,
    pub(crate) friction: Group<Kind, Friction>,
}

/// Per-kind outcomes are the trend's `outcomes`; this adds what became of
/// their pull requests, as the kept GitHub answer last showed.
#[derive(Serialize)]
pub(crate) struct Yield {
    pub(crate) pull_requests: Group<Kind, Survival>,
}

#[derive(Serialize)]
pub(crate) struct Quality {
    /// By the bucket each pull request merged in.
    pub(crate) bug_origins: Group<Kind, Cohort>,
    /// By each Scoring week's start.
    pub(crate) scores: Group<Criterion, CriterionScores>,
    pub(crate) attention: Group<Kind, Attention>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Kind {
    pub(crate) account: Option<String>,
    pub(crate) kind: &'static str,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Setup {
    pub(crate) account: Option<String>,
    pub(crate) kind: &'static str,
    pub(crate) harness: String,
    pub(crate) model: Option<String>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct Criterion {
    pub(crate) criterion: &'static str,
    pub(crate) calibrated: bool,
}

/// The values a line is compared and drawn by, the first one drawn. Names
/// ending in `_rate` or `_share` are fractions; those with `cost` are USD.
pub(crate) trait Headline {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)>;
}

pub(super) trait Measure<I>: Headline + Default {
    fn add(&mut self, item: &I);
}

/// One measure's lines, and the shown buckets their sparklines share.
#[derive(Serialize)]
pub(crate) struct Group<K, T> {
    pub(crate) axis: Vec<NaiveDate>,
    pub(crate) lines: Vec<Line<K, T>>,
}

#[derive(Serialize)]
pub(crate) struct Line<K, T> {
    #[serde(flatten)]
    pub(crate) key: K,
    /// The whole shown window.
    pub(crate) window: T,
    pub(crate) buckets: Vec<Point<T>>,
    pub(crate) comparison: Option<Compared<T>>,
}

#[derive(Serialize)]
pub(crate) struct Point<T> {
    pub(crate) bucket: NaiveDate,
    #[serde(flatten)]
    pub(crate) value: T,
}

/// The window against the preceding one. A change is missing unless both
/// sides have the value.
#[derive(Serialize)]
pub(crate) struct Compared<T> {
    pub(crate) current: T,
    pub(crate) prior: T,
    pub(crate) delta: BTreeMap<&'static str, Option<f64>>,
}

impl<K: Ord + Clone, T> Group<K, T> {
    fn of<I>(items: impl IntoIterator<Item = (NaiveDate, K, I)>, window: &Window) -> Self
    where
        T: Measure<I>,
    {
        let compare = window.current().zip(window.prior);
        let mut axis = BTreeSet::new();
        let mut lines = BTreeMap::<K, (T, BTreeMap<NaiveDate, T>, Option<(T, T)>)>::new();
        for (bucket, key, item) in items {
            let shown = window.shows(bucket);
            let side =
                compare.map(|(current, prior)| (current.contains(bucket), prior.contains(bucket)));
            if !shown && !side.is_some_and(|(_, prior)| prior) {
                continue;
            }
            let (whole, buckets, compared) = lines.entry(key).or_insert_with(|| {
                (
                    T::default(),
                    BTreeMap::new(),
                    compare.map(|_| Default::default()),
                )
            });
            if shown {
                axis.insert(bucket);
                whole.add(&item);
                buckets.entry(bucket).or_default().add(&item);
            }
            if let (Some((current, prior)), Some((in_current, in_prior))) = (compared, side) {
                if in_current {
                    current.add(&item);
                } else if in_prior {
                    prior.add(&item);
                }
            }
        }
        Self {
            axis: axis.into_iter().collect(),
            lines: lines
                .into_iter()
                .map(|(key, (window, buckets, compared))| Line {
                    key,
                    window,
                    buckets: buckets
                        .into_iter()
                        .map(|(bucket, value)| Point { bucket, value })
                        .collect(),
                    comparison: compared.map(|(current, prior)| Compared::of(current, prior)),
                })
                .collect(),
        }
    }
}

impl<T: Headline> Compared<T> {
    fn of(current: T, prior: T) -> Self {
        let before: BTreeMap<_, _> = prior.headline().into_iter().collect();
        let delta = current
            .headline()
            .into_iter()
            .map(|(name, now)| {
                let then = before.get(name).copied().flatten();
                (name, now.zip(then).map(|(now, then)| now - then))
            })
            .collect();
        Self {
            current,
            prior,
            delta,
        }
    }

    /// `minutes per success -3.5; wasted time share +10 pts`.
    pub(crate) fn text(&self) -> String {
        self.current
            .headline()
            .into_iter()
            .map(|(name, _)| {
                let change = self.delta.get(name).copied().flatten();
                format!("{} {}", label(name), change_text(name, change))
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A static sparkline: an SVG path over a 100 × 20 box, and its values in words.
pub(crate) struct Spark {
    pub(crate) path: String,
    pub(crate) label: String,
}

impl<K, T: Headline> Line<K, T> {
    pub(crate) fn spark(&self, axis: &[NaiveDate]) -> Spark {
        let name = self.window.headline().first().map_or("", |(name, _)| *name);
        let values: Vec<_> = axis
            .iter()
            .map(|bucket| {
                let value = self
                    .buckets
                    .iter()
                    .find(|point| point.bucket == *bucket)
                    .and_then(|point| point.value.headline().first().and_then(|(_, v)| *v));
                (*bucket, value)
            })
            .collect();
        spark(name, &values)
    }
}

fn label(name: &str) -> String {
    name.replace('_', " ")
}

fn fraction(name: &str) -> bool {
    name.ends_with("_rate") || name.ends_with("_share")
}

fn value_text(name: &str, value: Option<f64>) -> String {
    match value {
        None => "missing".into(),
        Some(value) if fraction(name) => format!("{:.0}%", value * 100.0),
        Some(value) if name.contains("cost") => format!("${value:.2}"),
        Some(value) if value.fract() == 0.0 => format!("{value:.0}"),
        Some(value) => format!("{value:.1}"),
    }
}

fn change_text(name: &str, change: Option<f64>) -> String {
    match change {
        None => "missing".into(),
        Some(change) if fraction(name) => format!("{:+.0} pts", change * 100.0),
        Some(change) if name.contains("cost") => format!("{change:+.2}"),
        Some(change) if change.fract() == 0.0 => format!("{change:+.0}"),
        Some(change) => format!("{change:+.1}"),
    }
}

fn spark(name: &str, values: &[(NaiveDate, Option<f64>)]) -> Spark {
    let top = values
        .iter()
        .filter_map(|(_, value)| *value)
        .fold(0.0, f64::max);
    let step = if values.len() > 1 {
        100.0 / (values.len() - 1) as f64
    } else {
        0.0
    };
    let mut path = Vec::new();
    let mut drawing = false;
    for (index, (_, value)) in values.iter().enumerate() {
        let Some(value) = value else {
            drawing = false;
            continue;
        };
        let x = if values.len() > 1 {
            index as f64 * step
        } else {
            50.0
        };
        let y = if top > 0.0 {
            18.0 - 16.0 * value / top
        } else {
            18.0
        };
        path.push(format!("{}{x:.2} {y:.2}", if drawing { "L" } else { "M" }));
        drawing = true;
    }
    let readings = values
        .iter()
        .map(|(bucket, value)| format!("{bucket} {}", value_text(name, *value)))
        .collect::<Vec<_>>()
        .join(", ");
    Spark {
        path: path.join(" "),
        label: format!("{}: {readings}", label(name)),
    }
}

fn ratio(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| part / whole)
}

/// What the kept GitHub answer last showed of a pull request.
#[derive(Clone, Copy)]
pub(super) enum Fate {
    Merged,
    Closed,
    Open,
    NotKept,
}

#[derive(Default, Serialize)]
pub(crate) struct Survival {
    #[serde(skip)]
    fates: BTreeMap<String, Fate>,
    pub(crate) merged: usize,
    /// Closed without merging.
    pub(crate) closed: usize,
    pub(crate) open: usize,
    /// No kept answer for it on this machine.
    pub(crate) not_kept: usize,
    /// Merged / (merged + closed).
    pub(crate) survival_rate: Option<f64>,
}

impl Measure<(String, Fate)> for Survival {
    fn add(&mut self, (url, fate): &(String, Fate)) {
        self.fates.entry(url.clone()).or_insert(*fate);
        let count = |wanted: fn(Fate) -> bool| self.fates.values().filter(|f| wanted(**f)).count();
        self.merged = count(|fate| matches!(fate, Fate::Merged));
        self.closed = count(|fate| matches!(fate, Fate::Closed));
        self.open = count(|fate| matches!(fate, Fate::Open));
        self.not_kept = count(|fate| matches!(fate, Fate::NotKept));
        self.survival_rate = ratio(self.merged as f64, (self.merged + self.closed) as f64);
    }
}

impl Survival {
    pub(crate) fn rate(&self) -> String {
        value_text("survival_rate", self.survival_rate)
    }
}

impl Headline for Survival {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![
            ("survival_rate", self.survival_rate),
            ("merged", Some(self.merged as f64)),
            ("closed_unmerged", Some(self.closed as f64)),
        ]
    }
}

/// Merged pull requests and the confirmed Bug origins that name them.
#[derive(Default, Serialize)]
pub(crate) struct Cohort {
    #[serde(skip)]
    origins: BTreeMap<String, usize>,
    pub(crate) merged: usize,
    pub(crate) confirmed_bug_origins: usize,
    pub(crate) bug_origins_per_merged: Option<f64>,
}

impl Measure<(String, usize)> for Cohort {
    fn add(&mut self, (url, origins): &(String, usize)) {
        *self.origins.entry(url.clone()).or_default() += origins;
        self.merged = self.origins.len();
        self.confirmed_bug_origins = self.origins.values().sum();
        self.bug_origins_per_merged = ratio(self.confirmed_bug_origins as f64, self.merged as f64);
    }
}

impl Cohort {
    pub(crate) fn per_merged(&self) -> String {
        value_text("bug_origins_per_merged", self.bug_origins_per_merged)
    }
}

impl Headline for Cohort {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![
            (
                "confirmed_bug_origins",
                Some(self.confirmed_bug_origins as f64),
            ),
            ("bug_origins_per_merged", self.bug_origins_per_merged),
        ]
    }
}

/// A Grader's Scores on one criterion. The pass rate is given only once
/// the criterion is calibrated.
#[derive(Default, Serialize)]
pub(crate) struct CriterionScores {
    pub(crate) graded: usize,
    pub(crate) passed: usize,
    pub(crate) pass_rate: Option<f64>,
}

/// One Score: whether it passed, and whether its criterion is calibrated.
pub(super) struct Scored {
    passed: bool,
    calibrated: bool,
}

impl Measure<Scored> for CriterionScores {
    fn add(&mut self, scored: &Scored) {
        self.graded += 1;
        self.passed += usize::from(scored.passed);
        self.pass_rate =
            ratio(self.passed as f64, self.graded as f64).filter(|_| scored.calibrated);
    }
}

impl CriterionScores {
    pub(crate) fn rate(&self) -> String {
        value_text("pass_rate", self.pass_rate)
    }
}

impl Headline for CriterionScores {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![("pass_rate", self.pass_rate)]
    }
}

/// Commands that ended needing the Day shift, and Stops asked of them.
#[derive(Default, Serialize)]
pub(crate) struct Attention {
    pub(crate) commands: usize,
    /// Failed, died, Paused or Stopped, or holding Security findings.
    pub(crate) needs_you: usize,
    pub(crate) stops: usize,
}

pub(super) struct Asked {
    needs_you: bool,
    stop: bool,
}

impl Measure<Asked> for Attention {
    fn add(&mut self, asked: &Asked) {
        self.commands += 1;
        self.needs_you += usize::from(asked.needs_you);
        self.stops += usize::from(asked.stop);
    }
}

impl Headline for Attention {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![
            ("needs_you", Some(self.needs_you as f64)),
            ("stops", Some(self.stops as f64)),
        ]
    }
}

/// Speed and Repair churn over Succeeded Commands only, and what every ended
/// attempt cost per succeeded delivery. Wasted is Failed (died included) and
/// Stopped; Paused work can still resume.
#[derive(Default, Serialize)]
pub(crate) struct Efficiency {
    pub(crate) ended: usize,
    pub(crate) succeeded: usize,
    pub(crate) succeeded_minutes: Signal,
    pub(crate) succeeded_repairs: Signal,
    pub(crate) succeeded_ci_rounds: Signal,
    /// Succeeded with no Repair, out of those with Repairs observed.
    pub(crate) first_pass_clean: usize,
    pub(crate) first_pass_clean_rate: Option<f64>,
    pub(crate) attempt_minutes: Signal,
    pub(crate) attempt_cost_usd: Signal,
    /// Observed attempt minutes and spend over succeeded Commands.
    pub(crate) minutes_per_success: Option<f64>,
    pub(crate) cost_usd_per_success: Option<f64>,
    pub(crate) wasted_minutes: Signal,
    pub(crate) wasted_cost_usd: Signal,
    /// Wasted over observed attempt totals.
    pub(crate) wasted_time_share: Option<f64>,
    pub(crate) wasted_spend_share: Option<f64>,
}

pub(super) struct Attempt {
    class: Class,
    minutes: Option<f64>,
    cost_usd: Option<f64>,
    repairs: Option<usize>,
    ci_rounds: Option<usize>,
}

impl Measure<Attempt> for Efficiency {
    fn add(&mut self, attempt: &Attempt) {
        self.ended += 1;
        self.attempt_minutes.add(attempt.minutes);
        self.attempt_cost_usd.add(attempt.cost_usd);
        match attempt.class {
            Class::Succeeded => {
                self.succeeded += 1;
                self.succeeded_minutes.add(attempt.minutes);
                self.succeeded_repairs
                    .add(attempt.repairs.map(|n| n as f64));
                self.succeeded_ci_rounds
                    .add(attempt.ci_rounds.map(|n| n as f64));
                self.first_pass_clean += usize::from(attempt.repairs == Some(0));
            }
            Class::Failed | Class::Stopped => {
                self.wasted_minutes.add(attempt.minutes);
                self.wasted_cost_usd.add(attempt.cost_usd);
            }
            Class::Paused | Class::Live => {}
        }
        self.first_pass_clean_rate = ratio(
            self.first_pass_clean as f64,
            self.succeeded_repairs.observed as f64,
        );
        let per_success =
            |total: &Signal| total.sum.and_then(|sum| ratio(sum, self.succeeded as f64));
        self.minutes_per_success = per_success(&self.attempt_minutes);
        self.cost_usd_per_success = per_success(&self.attempt_cost_usd);
        let share = |wasted: &Signal, total: &Signal| {
            total
                .sum
                .and_then(|sum| ratio(wasted.sum.unwrap_or(0.0), sum))
        };
        self.wasted_time_share = share(&self.wasted_minutes, &self.attempt_minutes);
        self.wasted_spend_share = share(&self.wasted_cost_usd, &self.attempt_cost_usd);
    }
}

/// `12.5 (3 observed, 1 missing)`.
fn covered(name: &str, value: Option<f64>, signal: &Signal) -> String {
    format!(
        "{} ({} observed, {} missing)",
        value_text(name, value),
        signal.observed,
        signal.missing
    )
}

impl Efficiency {
    /// `1/2 (50%)`.
    pub(crate) fn first_pass(&self) -> String {
        format!(
            "{}/{} ({})",
            self.first_pass_clean,
            self.succeeded_repairs.observed,
            value_text("first_pass_clean_rate", self.first_pass_clean_rate)
        )
    }

    pub(crate) fn minutes_per_success_text(&self) -> String {
        covered("minutes", self.minutes_per_success, &self.attempt_minutes)
    }

    pub(crate) fn cost_per_success_text(&self) -> String {
        covered(
            "cost_usd",
            self.cost_usd_per_success,
            &self.attempt_cost_usd,
        )
    }

    pub(crate) fn wasted_time_text(&self) -> String {
        covered(
            "wasted_time_share",
            self.wasted_time_share,
            &self.wasted_minutes,
        )
    }

    pub(crate) fn wasted_spend_text(&self) -> String {
        covered(
            "wasted_spend_share",
            self.wasted_spend_share,
            &self.wasted_cost_usd,
        )
    }
}

impl Headline for Efficiency {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![
            ("minutes_per_success", self.minutes_per_success),
            ("cost_usd_per_success", self.cost_usd_per_success),
            ("first_pass_clean_rate", self.first_pass_clean_rate),
            ("wasted_time_share", self.wasted_time_share),
            ("wasted_spend_share", self.wasted_spend_share),
            ("succeeded_minutes", self.succeeded_minutes.mean),
            ("succeeded_repairs", self.succeeded_repairs.mean),
            ("succeeded_ci_rounds", self.succeeded_ci_rounds.mean),
        ]
    }
}

/// Stage times above the repository's Usual range, and per-session friction.
#[derive(Default, Serialize)]
pub(crate) struct Friction {
    pub(crate) runs: usize,
    pub(crate) stages: BTreeMap<String, Above>,
    /// Every stage with a Usual range.
    #[serde(flatten)]
    pub(crate) all_stages: Above,
    pub(crate) sessions: usize,
    pub(crate) missing_sessions: usize,
    pub(crate) repeated_tool_calls: Signal,
    pub(crate) tool_errors: Signal,
    pub(crate) permission_denials: Signal,
    pub(crate) killed_work: Signal,
    /// Sessions with any destructive command, out of those checked.
    pub(crate) destructive_sessions: usize,
    pub(crate) destructive_checked: usize,
    pub(crate) destructive_incidence_rate: Option<f64>,
    /// Commands left with unaddressed Security findings.
    pub(crate) security_holds: usize,
    pub(crate) security_observed: usize,
    pub(crate) security_missing: usize,
}

/// Stages past the top of their Usual range, out of those that had one.
/// Suspended time already keeps a stage out of its range.
#[derive(Default, Serialize)]
pub(crate) struct Above {
    pub(crate) above: usize,
    pub(crate) ranged: usize,
    pub(crate) above_share: Option<f64>,
}

impl Above {
    fn add(&mut self, position: Position) {
        if position == Position::TooFewRuns {
            return;
        }
        self.ranged += 1;
        self.above += usize::from(position == Position::Above);
        self.above_share = ratio(self.above as f64, self.ranged as f64);
    }

    /// `2/8 (25%)`.
    pub(crate) fn text(&self) -> String {
        format!(
            "{}/{} ({})",
            self.above,
            self.ranged,
            value_text("above_share", self.above_share)
        )
    }
}

pub(super) struct Trajectory<'a> {
    stages: Vec<(&'a str, Position)>,
    sessions: Vec<Option<&'a Signals>>,
    security: Option<usize>,
}

impl Measure<Trajectory<'_>> for Friction {
    fn add(&mut self, run: &Trajectory<'_>) {
        self.runs += 1;
        for (name, position) in &run.stages {
            self.stages
                .entry((*name).to_owned())
                .or_default()
                .add(*position);
            self.all_stages.add(*position);
        }
        for session in &run.sessions {
            let Some(signals) = session else {
                self.missing_sessions += 1;
                continue;
            };
            self.sessions += 1;
            self.repeated_tool_calls
                .add(Some(signals.repeated_tool_calls as f64));
            self.tool_errors.add(Some(signals.tool_errors as f64));
            self.permission_denials
                .add(Some(signals.permission_denials as f64));
            self.killed_work.add(Some(signals.killed_work as f64));
            if let Some(flags) = &signals.destructive_commands {
                self.destructive_checked += 1;
                self.destructive_sessions += usize::from(!flags.is_empty());
            }
        }
        self.destructive_incidence_rate = ratio(
            self.destructive_sessions as f64,
            self.destructive_checked as f64,
        );
        match run.security {
            Some(count) => {
                self.security_observed += 1;
                self.security_holds += usize::from(count > 0);
            }
            None => self.security_missing += 1,
        }
    }
}

impl Friction {
    /// `implement 1/4 (25%), CI watch 0/3 (0%)`.
    pub(crate) fn stages_text(&self) -> String {
        self.stages
            .iter()
            .map(|(name, above)| format!("{name} {}", above.text()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// `1/6 (17%)`: sessions with any destructive command, of those checked.
    pub(crate) fn destructive(&self) -> String {
        format!(
            "{}/{} ({})",
            self.destructive_sessions,
            self.destructive_checked,
            value_text(
                "destructive_incidence_rate",
                self.destructive_incidence_rate
            )
        )
    }

    /// `1 (5 observed, 2 missing)`.
    pub(crate) fn security(&self) -> String {
        format!(
            "{} ({} observed, {} missing)",
            self.security_holds, self.security_observed, self.security_missing
        )
    }
}

impl Headline for Friction {
    fn headline(&self) -> Vec<(&'static str, Option<f64>)> {
        vec![
            ("above_usual_range_share", self.all_stages.above_share),
            ("repeated_tool_calls", self.repeated_tool_calls.mean),
            ("tool_errors", self.tool_errors.mean),
            ("permission_denials", self.permission_denials.mean),
            ("killed_work", self.killed_work.mean),
            (
                "destructive_incidence_rate",
                self.destructive_incidence_rate,
            ),
            ("security_holds", Some(self.security_holds as f64)),
        ]
    }
}

pub(super) fn read(facts: &[Fact<'_>], window: &Window, sources: &Sources<'_>) -> Groups {
    let kind = |fact: &Fact<'_>| Kind {
        account: fact.counted.account.clone(),
        kind: fact.counted.kind,
    };
    let ended = || {
        facts
            .iter()
            .filter(|fact| fact.counted.class != Class::Live)
    };
    let pull_requests = || {
        ended().filter_map(|fact| {
            let url = fact.command.record.pull_request.as_deref()?;
            let key = super::super::github::pull_request_key(url);
            let kept = sources.kept.get(&key);
            Some((fact, key, kept))
        })
    };
    let survival = pull_requests().map(|(fact, key, kept)| {
        let fate = match kept.map(|kept| kept.state) {
            Some(PrState::Merged) => Fate::Merged,
            Some(PrState::Closed) => Fate::Closed,
            Some(PrState::Open) => Fate::Open,
            None => Fate::NotKept,
        };
        (fact.counted.bucket, kind(fact), (key, fate))
    });
    let cohorts = pull_requests().filter_map(|(fact, key, kept)| {
        let kept = kept.filter(|kept| kept.state == PrState::Merged)?;
        let merged = DateTime::parse_from_rfc3339(kept.merged_at.as_deref()?).ok()?;
        let bucket = window.bucket_on(merged.with_timezone(&Utc).date_naive());
        Some((
            bucket,
            kind(fact),
            (key, sources.origins.for_run(fact.command)),
        ))
    });
    let attention = facts.iter().map(|fact| {
        let needs_you = matches!(
            fact.counted.class,
            Class::Failed | Class::Paused | Class::Stopped
        ) || security(fact.command).is_some_and(|count| count > 0)
            || fact.command.record.security_findings_waiting;
        (
            fact.counted.bucket,
            kind(fact),
            Asked {
                needs_you,
                stop: fact.stop_asked,
            },
        )
    });
    let efficiency = ended().map(|fact| {
        let metrics = fact.command.record.run_metrics.as_ref();
        (
            fact.counted.bucket,
            Setup {
                account: fact.counted.account.clone(),
                kind: fact.counted.kind,
                harness: fact.command.record.harness.clone(),
                model: fact.command.record.model.clone(),
            },
            Attempt {
                class: fact.counted.class,
                minutes: attempt_minutes(fact.command),
                cost_usd: cost_usd(&sessions(fact.command)),
                repairs: metrics.map(|metrics| metrics.repairs),
                ci_rounds: metrics.map(|metrics| metrics.ci_rounds),
            },
        )
    });
    let friction = ended().filter_map(|fact| {
        let metrics = fact.command.record.run_metrics.as_ref()?;
        Some((
            fact.counted.bucket,
            kind(fact),
            Trajectory {
                stages: metrics
                    .stages
                    .iter()
                    .map(|stage| (stage.name.as_str(), stage.position))
                    .collect(),
                sessions: sessions(fact.command),
                security: security(fact.command),
            },
        ))
    });
    Groups {
        yield_: Yield {
            pull_requests: Group::of(survival, window),
        },
        quality: Quality {
            bug_origins: Group::of(cohorts, window),
            scores: Group::of(scores(sources, window), window),
            attention: Group::of(attention, window),
        },
        efficiency: Group::of(efficiency, window),
        friction: Group::of(friction, window),
    }
}

fn scores(sources: &Sources<'_>, window: &Window) -> Vec<(NaiveDate, Criterion, Scored)> {
    let mut items = Vec::new();
    for week in &sources.scores.weeks {
        let bucket = window.bucket_on(week.start.date_naive());
        for pr in &week.pull_requests {
            if !sources
                .filters
                .admit_scored(&pr.repository, &pr.harness, &pr.model)
            {
                continue;
            }
            for (criterion, passed) in &pr.scores {
                let calibrated = sources
                    .scores
                    .calibrated
                    .get(criterion)
                    .copied()
                    .unwrap_or(false);
                items.push((
                    bucket,
                    Criterion {
                        criterion: criterion.name(),
                        calibrated,
                    },
                    Scored {
                        passed: *passed,
                        calibrated,
                    },
                ));
            }
        }
    }
    items
}

fn security(command: &Command) -> Option<usize> {
    command
        .record
        .run_metrics
        .as_ref()?
        .unaddressed_findings
        .as_ref()?
        .security
}

/// A Command's time from start to end. One that died has no end, so its
/// Command log's last write stands in for it.
fn attempt_minutes(command: &Command) -> Option<f64> {
    if let Some(metrics) = &command.record.run_metrics {
        return metrics.total_minutes;
    }
    if command.state != "died" {
        return None;
    }
    let began = DateTime::parse_from_rfc3339(&command.record.started_at).ok()?;
    let last: DateTime<Utc> = std::fs::metadata(&command.command_log)
        .ok()?
        .modified()
        .ok()?
        .into();
    let milliseconds = (last - began.with_timezone(&Utc)).num_milliseconds();
    (milliseconds >= 0).then(|| milliseconds as f64 / 60_000.0)
}

/// Each distinct Session's signals, missing where its trace gave none. A
/// Resume continues its original Session and is not counted again.
fn sessions(command: &Command) -> Vec<Option<&Signals>> {
    let Some(metrics) = &command.record.run_metrics else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    metrics
        .stages
        .iter()
        .enumerate()
        .filter(|(_, stage)| StageName::is_session(&stage.name))
        .filter(|(index, stage)| {
            command
                .record
                .stages
                .get(*index)
                .filter(|recorded| recorded.name == stage.name)
                .and_then(|recorded| recorded.session_log.as_deref())
                .is_none_or(|log| seen.insert(super::super::session::original(log)))
        })
        .map(|(_, stage)| {
            stage
                .session_signals
                .as_ref()
                .map(|session| &session.signals)
        })
        .collect()
}

/// Observed or priced spend across every Session, missing unless each has one.
fn cost_usd(sessions: &[Option<&Signals>]) -> Option<f64> {
    if sessions.is_empty() {
        return None;
    }
    sessions
        .iter()
        .map(|session| session.and_then(|signals| signals.cost_usd))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attempt(class: Class, minutes: f64, repairs: usize) -> Attempt {
        Attempt {
            class,
            minutes: Some(minutes),
            cost_usd: Some(minutes / 10.0),
            repairs: Some(repairs),
            ci_rounds: Some(1 + repairs),
        }
    }

    #[test]
    fn crashes_raise_waste_without_flattering_succeeded_means() {
        let mut efficiency = Efficiency::default();
        efficiency.add(&attempt(Class::Succeeded, 30.0, 0));
        efficiency.add(&attempt(Class::Succeeded, 50.0, 2));
        let before = (
            efficiency.succeeded_minutes.mean,
            efficiency.succeeded_repairs.mean,
            efficiency.succeeded_ci_rounds.mean,
        );
        assert_eq!(efficiency.wasted_time_share, Some(0.0));
        for _ in 0..4 {
            efficiency.add(&attempt(Class::Failed, 0.5, 0));
        }
        efficiency.add(&attempt(Class::Stopped, 2.0, 0));
        assert_eq!(
            (
                efficiency.succeeded_minutes.mean,
                efficiency.succeeded_repairs.mean,
                efficiency.succeeded_ci_rounds.mean,
            ),
            before
        );
        assert_eq!(efficiency.first_pass_clean_rate, Some(0.5));
        assert_eq!(efficiency.minutes_per_success, Some(42.0));
        for share in [efficiency.wasted_time_share, efficiency.wasted_spend_share] {
            assert!((share.unwrap() - 4.0 / 84.0).abs() < 1e-12, "{share:?}");
        }
    }

    #[test]
    fn a_missing_session_cost_leaves_the_commands_spend_missing() {
        let priced = Signals {
            cost_usd: Some(1.5),
            ..Signals::default()
        };
        assert_eq!(cost_usd(&[Some(&priced), Some(&priced)]), Some(3.0));
        assert_eq!(cost_usd(&[Some(&priced), None]), None);
        assert_eq!(cost_usd(&[]), None);
    }

    #[test]
    fn sparklines_break_at_missing_buckets_and_say_their_values() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let drawn = spark(
            "wasted_time_share",
            &[(day(1), Some(0.5)), (day(2), None), (day(3), Some(0.25))],
        );
        assert_eq!(drawn.path, "M0.00 2.00 M100.00 10.00");
        assert_eq!(
            drawn.label,
            "wasted time share: 2026-11-01 50%, 2026-11-02 missing, 2026-11-03 25%"
        );
    }

    #[test]
    fn deltas_need_both_windows() {
        let mut current = Attention::default();
        current.add(&Asked {
            needs_you: true,
            stop: true,
        });
        let compared = Compared::of(current, Attention::default());
        assert_eq!(compared.delta["needs_you"], Some(1.0));
        assert_eq!(compared.text(), "needs you +1; stops +1");
        let mut current = CriterionScores::default();
        current.add(&Scored {
            passed: true,
            calibrated: true,
        });
        let compared = Compared::of(current, CriterionScores::default());
        assert_eq!(compared.text(), "pass rate missing");
    }
}
