//! Local, read-only metrics views; every signal retains its own units.

mod trends;

use anyhow::{Result, bail};
use chrono::Local;
use serde::Serialize;
use serde_json::Value;

use super::reading::{Command, Observation, Snapshot};
use super::run_metrics::RunMetrics;
use crate::args::MetricsArgs;
use crate::config::UserConfig;

#[derive(Serialize)]
struct Run<'a> {
    name: &'a str,
    kind: &'a str,
    repository: &'a str,
    harness: &'a str,
    model: Option<&'a str>,
    effort: Option<&'a str>,
    started_at: &'a str,
    metrics: Option<&'a RunMetrics>,
    children: Vec<Run<'a>>,
}

impl<'a> Run<'a> {
    fn of(command: &'a Command) -> Self {
        let record = &command.record;
        Self {
            name: &record.name,
            kind: &record.kind,
            repository: &record.repository,
            harness: &record.harness,
            model: record.model.as_deref(),
            effort: record.effort.as_deref(),
            started_at: &record.started_at,
            metrics: record.run_metrics.as_ref(),
            children: command
                .dispatched
                .iter()
                .chain(&command.children)
                .map(Self::of)
                .collect(),
        }
    }
}

pub fn show(asked: &MetricsArgs) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load_metrics(&config.logs_dir, Local::now())?;
    let mut report = serde_json::json!({
        "version": 1,
        "hostname": crate::host::name().unwrap_or_else(|| "unknown".into())
    });
    if let Some(input) = &asked.command {
        let Observation::Record(command) = snapshot.resolve(input)?.observation else {
            bail!("metrics need a Command record; this Command has only logs");
        };
        report["command"] = serde_json::to_value(Run::of(command))?;
    } else {
        report["weeks"] = serde_json::to_value(trends::weekly(&snapshot, asked))?;
    }
    if asked.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print(&report, 0);
    }
    Ok(())
}

// Both renderers walk the same document, so text cannot silently drop a signal.
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

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "missing".into(),
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
