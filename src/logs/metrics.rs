//! Local, read-only metrics views; every signal retains its own units.

mod bug_origins;
mod trends;

use anyhow::{Result, bail};
use chrono::Local;
use serde::Serialize;
use serde_json::Value;

use super::reading::{Command, Observation, Snapshot};
use super::run_metrics::RunMetrics;
use crate::args::MetricsArgs;
use crate::config::UserConfig;

use bug_origins::ConfirmedCounts;

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
    started_at: &'a str,
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
            started_at: &record.started_at,
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
    let snapshot = Snapshot::load_metrics(&config.logs_dir, Local::now())?;
    let origins = ConfirmedCounts::read(&snapshot)?;
    let mut report = serde_json::json!({
        "version": 1,
        "hostname": crate::host::name().unwrap_or_else(|| "unknown".into())
    });
    if let Some(input) = &asked.command {
        let Observation::Record(command) = snapshot.resolve(input)?.observation else {
            bail!("metrics need a Command record; this Command has only logs");
        };
        report["command"] = serde_json::to_value(Run::of(command, &origins))?;
    } else {
        report["weeks"] = serde_json::to_value(trends::weekly(&snapshot, asked, &origins))?;
    }
    if asked.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print(&report, 0);
    }
    Ok(())
}

// Both renderers walk the same document, so text cannot silently drop a signal.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "objects and arrays nest; every other JSON value prints on its line as a scalar"
)]
fn print(value: &Value, depth: usize) {
    let indent = "  ".repeat(depth);
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
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

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "JSON values without their own wording print as JSON shows them"
)]
fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "missing".into(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
