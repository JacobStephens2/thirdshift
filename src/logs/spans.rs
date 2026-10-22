//! One retained span tree for the CLI and Console. Presentation never has to
//! recover record ancestry, match Harness events, or deduplicate Sessions.
mod session;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::Value;

use super::reading::{Command, Entry, Observation, Snapshot};
use super::status::{inspect, waterfall};
use super::trace::{Context, id};
use crate::config::UserConfig;

#[derive(Clone, Serialize)]
pub(crate) struct Span {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub name: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub duration_ms: Option<f64>,
    pub status: String,
    pub error: Option<String>,
    pub estimated: bool,
    pub attributes: BTreeMap<String, Value>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub command: String,
    pub command_url: String,
    pub conversation_url: Option<String>,
    pub run: Option<u64>,
    pub session_kind: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct Tree {
    pub command: String,
    pub spans: Vec<Span>,
    pub now: f64,
}

#[derive(Serialize)]
pub(crate) struct Choice {
    pub subject: String,
    pub state: String,
    pub url: String,
    pub selected: bool,
}

pub(crate) struct View {
    pub choices: Vec<Choice>,
    pub tree: Tree,
}

pub(crate) fn show(input: &str, selected: Option<&str>, json: bool) -> Result<()> {
    let view = load(&UserConfig::load()?, input)?;
    if let Some(selected) = selected {
        let span = view.tree.span(selected)?;
        if json {
            println!("{}", serde_json::to_string_pretty(span)?);
        } else {
            println!("{} {} · {}", span.kind, span.name, span.status);
            println!("{}", serde_json::to_string_pretty(span)?);
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(&view.tree)?);
    } else {
        for span in &view.tree.spans {
            println!("{} {} {} · {}", span.id, span.kind, span.name, span.status);
        }
    }
    Ok(())
}

impl Tree {
    pub(crate) fn span(&self, selected: &str) -> Result<&Span> {
        self.spans
            .iter()
            .find(|span| span.id == selected)
            .with_context(|| format!("span {selected} is not in this Command"))
    }
}

pub(crate) fn load(config: &UserConfig, input: &str) -> Result<View> {
    let now = Local::now();
    let snapshot = Snapshot::load_with_silence(&config.logs_dir, now, config.silent_interval)?;
    let mut commands: Vec<_> = snapshot
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            Entry::Record(command) => Some(command.as_ref()),
            _ => None,
        })
        .collect();
    commands.sort_by_key(|command| {
        (
            command.state != "live",
            std::cmp::Reverse(time(&command.record.started_at)),
        )
    });
    let (selected, owner) = if input.is_empty() {
        (commands.first().copied(), commands.first().copied())
    } else {
        let selection = snapshot.resolve(input)?;
        match selection.observation {
            Observation::Record(command) => (Some(command), selection.owner.or(Some(command))),
            _ => bail!("span tree requires a Command record"),
        }
    };
    let selector = selected
        .and_then(|command| Observation::Record(command).selector())
        .unwrap_or_default();
    let choices = commands
        .iter()
        .map(|command| {
            let choice = Observation::Record(command).selector().unwrap_or_default();
            Choice {
                subject: subject(command),
                state: state(command),
                url: inspect::url("/waterfall", &choice, None),
                selected: choice == selector,
            }
        })
        .collect();
    let mut tree = Tree {
        command: selector,
        spans: Vec::new(),
        now: now.timestamp_millis() as f64,
    };
    if let (Some(command), Some(owner)) = (selected, owner) {
        let context = Context::of_session(&owner.command_log);
        let root = record_span(command, hex(context.command), None, "cmd", &tree.command);
        tree.spans.push(root);
        let mut builder = Builder {
            context,
            sessions: BTreeSet::new(),
            log: waterfall::matching::Log::read(
                &owner.command_log,
                DateTime::parse_from_rfc3339(&owner.record.started_at)?,
            )?,
            now: now.fixed_offset(),
            spans: &mut tree.spans,
        };
        let mut relays = Vec::new();
        relay_path(owner, command, &mut relays);
        builder.record(command, hex(builder.context.command), &relays)?;
    }
    Ok(View { choices, tree })
}

struct Builder<'a> {
    context: Context,
    sessions: BTreeSet<PathBuf>,
    log: waterfall::matching::Log,
    now: DateTime<chrono::FixedOffset>,
    spans: &'a mut Vec<Span>,
}

impl Builder<'_> {
    fn record(&mut self, command: &Command, parent: String, relays: &[u64]) -> Result<()> {
        let context = self.context.for_record(&command.record);
        let selector = Observation::Record(command).selector().unwrap_or_default();
        let run_id = hex(context.run);
        self.spans.push(record_span(
            command,
            run_id.clone(),
            Some(parent),
            "run",
            &selector,
        ));
        let stages: Vec<_> = command.record.stage_times(None).collect();
        for (index, timing) in stages.iter().enumerate() {
            let mut span = record_span(
                command,
                hex(context.stage_id(timing.stage)),
                Some(run_id.clone()),
                "stage",
                &selector,
            );
            span.name = timing.stage.name.clone();
            span.start = timing.began.map(|time| time.timestamp_millis() as f64);
            span.end = timing.ended.map(|time| time.timestamp_millis() as f64);
            span.status = if span.end.is_some() {
                "ok"
            } else if command.state == "live" {
                "running"
            } else {
                "unknown"
            }
            .into();
            span.error = None;
            if index + 1 == stages.len() && command.record.cause.is_some() {
                span.error.clone_from(&command.record.cause);
                span.status = "error".into();
            }
            span.duration_ms = duration(span.start, span.end);
            self.spans.push(span);
        }
        for path in Observation::Record(command).sessions()? {
            let original = super::session::original(&path);
            if !self.sessions.insert(original.clone()) {
                continue;
            }
            let stage = stages.iter().find(|timing| {
                timing
                    .stage
                    .session_log
                    .as_deref()
                    .is_some_and(|reference| super::session::original(reference) == original)
            });
            let parent = stage.map_or_else(
                || run_id.clone(),
                |timing| hex(context.stage_id(timing.stage)),
            );
            let mut template = record_span(command, String::new(), Some(parent), "sess", &selector);
            template.start = stage
                .and_then(|timing| timing.began)
                .map(|time| time.timestamp_millis() as f64)
                .or(template.start);
            template.input = None;
            template.conversation_url = Some(inspect::url("/log", &selector, Some(&path)));
            if let Some(mut spans) = session::read(&path, &context, &template)? {
                if spans.iter().any(|span| {
                    span.attributes.get("thirdshift.migration")
                        == Some(&Value::String("legacy".into()))
                }) {
                    let (calls, measured) =
                        waterfall::session_calls(&path, command, relays, &mut self.log)?;
                    let placed = waterfall::place_calls(&path, command, &calls, measured, self.now);
                    for span in spans.iter_mut().filter(|span| span.kind == "tool") {
                        let call = span
                            .attributes
                            .get("gen_ai.tool.call.id")
                            .and_then(Value::as_str);
                        let parent = span
                            .attributes
                            .get("thirdshift.parent_id")
                            .and_then(Value::as_str);
                        if let Some(timing) = placed.iter().find(|timing| {
                            timing.identity.as_ref().is_some_and(|(scope, id)| {
                                Some(id.as_str()) == call && scope.as_deref() == parent
                            })
                        }) {
                            span.start = timing.start.map(|time| time.timestamp_millis() as f64);
                            span.end = timing.end.map(|time| time.timestamp_millis() as f64);
                            span.duration_ms = duration(span.start, span.end);
                            span.estimated = true;
                        }
                    }
                }
                self.spans.extend(spans);
            } else {
                // Old logs retain their scoped estimates without pretending to
                // contain Sessions or measured model calls they never recorded.
                let (calls, measured) =
                    waterfall::session_calls(&path, command, relays, &mut self.log)?;
                for (index, call) in
                    waterfall::place_calls(&path, command, &calls, measured, self.now)
                        .into_iter()
                        .enumerate()
                {
                    let mut span = template.clone();
                    span.id = hex(id(&[
                        &context.run.0,
                        original.as_os_str().as_encoded_bytes(),
                        &index.to_be_bytes(),
                    ]));
                    span.parent_id = call
                        .stage
                        .map(|index| hex(context.stage_id(stages[index].stage)))
                        .or(Some(run_id.clone()));
                    span.kind = "tool".into();
                    span.name = call.label;
                    span.start = call.start.map(|time| time.timestamp_millis() as f64);
                    span.end = call.end.map(|time| time.timestamp_millis() as f64);
                    span.duration_ms = duration(span.start, span.end);
                    span.status = if span.end.is_some() { "ok" } else { "unknown" }.into();
                    span.error = None;
                    span.estimated = true;
                    self.spans.push(span);
                }
            }
        }
        for child in command.dispatched.iter().chain(&command.children) {
            let parent = stages.iter().rev().find(|timing| {
                timing.began.is_some_and(|began| {
                    time(&child.record.started_at)
                        .is_some_and(|start| start >= began.timestamp_millis())
                })
            });
            let parent = parent.map_or_else(
                || run_id.clone(),
                |timing| hex(context.stage_id(timing.stage)),
            );
            let mut child_relays = relays.to_vec();
            if command
                .children
                .iter()
                .any(|entry| std::ptr::eq(entry, child))
                && let Some(issue) = &child.record.issue
            {
                child_relays.push(issue.number);
            }
            self.record(child, parent, &child_relays)?;
        }
        Ok(())
    }
}

fn record_span(
    command: &Command,
    id: String,
    parent_id: Option<String>,
    kind: &str,
    selector: &str,
) -> Span {
    let start = time(&command.record.started_at).map(|time| time as f64);
    let end = command
        .record
        .ended_at
        .as_deref()
        .and_then(time)
        .map(|time| time as f64);
    Span {
        id,
        parent_id,
        kind: kind.into(),
        name: format!(
            "{} · {}{}",
            command.record.kind,
            subject(command),
            command
                .record
                .issue
                .as_ref()
                .and_then(|issue| issue.title.as_ref())
                .map(|title| format!(" · {title}"))
                .unwrap_or_default()
        ),
        start,
        end,
        duration_ms: duration(start, end),
        status: state(command),
        error: command.record.cause.clone(),
        estimated: false,
        attributes: BTreeMap::new(),
        input: None,
        output: None,
        command: selector.into(),
        command_url: inspect::url("/peek", selector, None),
        conversation_url: None,
        run: command.record.issue.as_ref().map(|issue| issue.number),
        session_kind: None,
    }
}

fn subject(command: &Command) -> String {
    command
        .record
        .issue
        .as_ref()
        .map(|issue| format!("{}#{}", command.record.repository, issue.number))
        .unwrap_or_else(|| command.record.name.clone())
}

fn state(command: &Command) -> String {
    command.record.outcome.clone().unwrap_or_else(|| {
        if command.state == "live" {
            "running"
        } else {
            command.state
        }
        .into()
    })
}

fn relay_path(command: &Command, selected: &Command, path: &mut Vec<u64>) -> bool {
    if std::ptr::eq(command, selected) {
        return true;
    }
    for child in command.dispatched.iter().chain(&command.children) {
        let length = path.len();
        if command
            .children
            .iter()
            .any(|entry| std::ptr::eq(entry, child))
            && let Some(issue) = &child.record.issue
        {
            path.push(issue.number);
        }
        if relay_path(child, selected, path) {
            return true;
        }
        path.truncate(length);
    }
    false
}

fn time(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}
fn duration(start: Option<f64>, end: Option<f64>) -> Option<f64> {
    start
        .zip(end)
        .and_then(|(start, end)| (end >= start).then_some(end - start))
}
fn hex(id: crate::otlp_jsonl::SpanId) -> String {
    id.0.iter().map(|byte| format!("{byte:02x}")).collect()
}
