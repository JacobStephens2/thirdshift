//! Local, read-only metrics views; every signal retains its own units.

mod bug_origins;
mod failures;
pub(crate) mod fleet;
mod groups;
mod outcomes;
mod scope;
mod trends;
mod versions;

use std::collections::BTreeSet;

use anyhow::{Result, bail};
use chrono::{Local, Utc};
use serde::Serialize;
use serde_json::Value;

use super::reading::{Command, Observation, Snapshot};
use super::run_metrics::RunMetrics;
use crate::args::MetricsArgs;
use crate::config::UserConfig;

use bug_origins::ConfirmedCounts;
pub(crate) use failures::{Ending, Stopped, WhereRunsFailed};
pub(crate) use groups::Groups;
pub(crate) use outcomes::{Bucket, COUNTED, Class, Comparison};
pub(crate) use scope::{Filters, Window};
pub(crate) use trends::Row;
pub(crate) use versions::{Transition, Versions};

#[derive(Serialize)]
struct Metrics<'a> {
    #[serde(flatten)]
    completion: &'a RunMetrics,
    confirmed_bug_origins: usize,
}

impl<'a> Metrics<'a> {
    fn of(command: &'a Command, origins: &ConfirmedCounts) -> Option<Self> {
        command.record.run_metrics.as_ref().map(|completion| Self {
            completion,
            confirmed_bug_origins: origins.for_run(command),
        })
    }
}

#[derive(Serialize)]
struct Run<'a> {
    name: &'a str,
    kind: &'a str,
    repository: &'a str,
    harness: &'a str,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    thirdshift_version: Option<&'a str>,
    started_at: &'a str,
    stopped_at: Option<&'a str>,
    failure_category: Option<crate::logs::FailureCategory>,
    metrics: Option<Metrics<'a>>,
    children: Vec<Run<'a>>,
}

impl<'a> Run<'a> {
    fn of(command: &'a Command, origins: &ConfirmedCounts) -> Self {
        let record = &command.record;
        Self {
            name: &record.name,
            kind: &record.kind,
            repository: &record.repository,
            harness: &record.harness,
            model: record.model.as_deref(),
            effort: record.effort.as_deref(),
            thirdshift_version: record.thirdshift_version.as_deref(),
            started_at: &record.started_at,
            stopped_at: record.stopped_at.as_deref(),
            failure_category: record.failure_category,
            metrics: Metrics::of(command, origins),
            children: command
                .dispatched
                .iter()
                .chain(&command.children)
                .map(|child| Self::of(child, origins))
                .collect(),
        }
    }
}

/// CLI and Console details use the same read-time metrics as `metrics`.
/// Late Bug-origin decisions belong to this view, never the durable record.
pub(super) fn details(snapshot: &Snapshot, observation: Observation<'_>) -> Result<Value> {
    let mut details = serde_json::to_value(observation.details()?)?;
    if let Observation::Record(command) = observation {
        let origins = ConfirmedCounts::read(snapshot)?;
        add_details(&mut details, command, &origins)?;
    }
    Ok(details)
}

fn add_details(details: &mut Value, command: &Command, origins: &ConfirmedCounts) -> Result<()> {
    details["run_metrics"] = serde_json::to_value(Metrics::of(command, origins))?;
    for (field, children) in [
        ("children", &command.children),
        ("dispatched", &command.dispatched),
    ] {
        for (index, child) in children.iter().enumerate() {
            add_details(&mut details[field][index], child, origins)?;
        }
    }
    Ok(())
}

pub fn show(asked: &MetricsArgs) -> Result<()> {
    let config = UserConfig::load()?;
    let report = if reads_fleet(&config, asked) {
        let report = serde_json::to_value(fleet::Capture::new(&config, None).load(asked)?)?;
        if asked.json {
            report
        } else {
            // Another machine's text must not start a line of its own.
            one_line(report)
        }
    } else {
        read(&config, asked)?
    };
    if asked.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print(&report, 0);
    }
    Ok(())
}

/// Whether `metrics` reads Fleet machines: `--fleet`, or `--machine` naming
/// any machine but this one.
pub(crate) fn reads_fleet(config: &UserConfig, asked: &MetricsArgs) -> bool {
    asked.fleet
        || asked
            .machine
            .as_ref()
            .is_some_and(|name| *name != crate::console::fleet_control::local_name(config))
}

fn one_line(value: Value) -> Value {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "only text can hold a line break; other JSON values pass through"
    )]
    match value {
        Value::String(text) => Value::String(crate::cronjobs::report_line(&text)),
        Value::Array(values) => Value::Array(values.into_iter().map(one_line).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(name, value)| (crate::cronjobs::report_line(&name), one_line(value)))
                .collect(),
        ),
        value => value,
    }
}

/// Trends across every Command on this machine the serving account can read,
/// with Other-account Commands tagged by OS account.
#[derive(Serialize)]
pub(crate) struct Trends {
    pub(crate) version: u32,
    pub(crate) hostname: String,
    pub(crate) window: Window,
    pub(crate) filters: Filters,
    pub(crate) unavailable_accounts: BTreeSet<String>,
    pub(crate) outcomes: Vec<Bucket>,
    pub(crate) comparison: Option<Comparison>,
    pub(crate) where_runs_failed: WhereRunsFailed,
    pub(crate) versions: Versions,
    pub(crate) groups: Groups,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) weeks: Option<Vec<Row>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) days: Option<Vec<Row>>,
}

impl Trends {
    pub(crate) fn rows(&self) -> &[Row] {
        self.weeks
            .as_deref()
            .or(self.days.as_deref())
            .unwrap_or_default()
    }

    /// The outcome rows in bucket order, each bucket's transitions first.
    pub(crate) fn timeline(&self) -> Vec<Timeline<'_>> {
        let mut lines = Vec::new();
        let mut marked = None;
        for bucket in &self.outcomes {
            if marked != Some(bucket.bucket) {
                marked = Some(bucket.bucket);
                lines.extend(
                    self.versions
                        .transitions
                        .iter()
                        .filter(|transition| transition.bucket == bucket.bucket)
                        .map(Timeline::Transition),
                );
            }
            lines.push(Timeline::Bucket(bucket));
        }
        lines
    }
}

pub(crate) enum Timeline<'a> {
    Transition(&'a Transition),
    Bucket(&'a Bucket),
}

fn hostname() -> String {
    crate::host::name().unwrap_or_else(|| "unknown".into())
}

/// `asked` must name no Command; [`read`] shows one Command's metrics.
pub(crate) fn trends(config: &UserConfig, asked: &MetricsArgs) -> Result<Trends> {
    let now = Local::now();
    let snapshot = Snapshot::metrics_for_user(config, now)?;
    let origins = ConfirmedCounts::read(&snapshot)?;
    let window = Window::of(asked, now.with_timezone(&Utc).date_naive());
    let filters = Filters::of(asked);
    let serving = super::accounts::serving_name();
    let facts = outcomes::facts(&snapshot, &window, &filters, serving.as_deref());
    let outcomes = outcomes::read(&facts, &window);
    let where_runs_failed = failures::read(&facts, &window);
    let versions = versions::read(&facts, &window, &filters);
    let groups = groups::read(
        &facts,
        &window,
        &groups::Sources {
            origins: &origins,
            kept: &super::github::kept_pull_requests(&config.logs_dir),
            scores: &crate::scoring::read_scores()?,
            filters: &filters,
        },
    );
    let rows = trends::signals(
        &snapshot,
        &trends::Scope {
            window: &window,
            filters: &filters,
            serving: serving.as_deref(),
        },
        &origins,
    );
    let (weeks, days) = if window.weekly() {
        (Some(rows), None)
    } else {
        (None, Some(rows))
    };
    Ok(Trends {
        version: 1,
        hostname: hostname(),
        window,
        filters,
        unavailable_accounts: snapshot.unavailable_accounts.clone(),
        outcomes: outcomes.buckets,
        comparison: outcomes.comparison,
        where_runs_failed,
        versions,
        groups,
        weeks,
        days,
    })
}

pub(crate) fn read(config: &UserConfig, asked: &MetricsArgs) -> Result<Value> {
    let Some(input) = &asked.command else {
        return Ok(serde_json::to_value(trends(config, asked)?)?);
    };
    let snapshot = Snapshot::load_metrics(&config.logs_dir, Local::now())?;
    let origins = ConfirmedCounts::read(&snapshot)?;
    let Observation::Record(command) = snapshot.resolve(input)?.observation else {
        bail!("metrics need a Command record; this Command has only logs");
    };
    Ok(serde_json::json!({
        "version": 1,
        "hostname": hostname(),
        "command": Run::of(command, &origins),
    }))
}

// Both renderers walk the same document, so text cannot silently drop a signal.
fn print(value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "objects and arrays nest; every other JSON value prints on its line as a scalar"
    )]
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "objects and arrays nest; every other JSON value prints on its line as a scalar"
                )]
                match value {
                    Value::Object(_) | Value::Array(_) => {
                        println!("{indent}{name}:");
                        print(value, depth + 1);
                    }
                    _ => println!("{indent}{name}: {}", scalar(value)),
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                println!("{indent}-");
                print(value, depth + 1);
            }
        }
        _ => println!("{indent}{}", scalar(value)),
    }
}

fn scalar(value: &Value) -> String {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "JSON values without their own wording print as JSON shows them"
    )]
    match value {
        Value::Null => "missing".into(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
