//! One retained span tree for the CLI and Console. Presentation never has to
//! recover record ancestry, match Harness events, or deduplicate Sessions.
mod session;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::Value;

use super::reading::{Command, Entry, Observation, Snapshot};
use super::status::inspect;
use super::trace::SpanKind;
use super::trace::ancestry::{self, Fact, Node};
use crate::config::UserConfig;

#[derive(Clone, Serialize)]
pub(crate) struct Span {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: SpanKind,
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

impl Span {
    fn display_name(&self) -> String {
        match self.kind {
            SpanKind::Cmd | SpanKind::Run => self.name.split_once(" · ").map_or_else(
                || self.name.clone(),
                |(kind, subject)| format!("{} · {subject}", super::display_name(kind)),
            ),
            SpanKind::Stage => super::display_name(&self.name).to_owned(),
            _ => self.name.clone(),
        }
    }
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
    pub title: Option<String>,
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
            println!("{} {} · {}", span.kind, span.display_name(), span.status);
            println!("{}", serde_json::to_string_pretty(span)?);
        }
    } else if json {
        println!("{}", serde_json::to_string_pretty(&view.tree)?);
    } else {
        for span in &view.tree.spans {
            println!(
                "{} {} {} · {}",
                span.id,
                span.kind,
                span.display_name(),
                span.status
            );
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

    /// A Session repeats its children's usage. Prefer a retained total at each
    /// level, falling back to children only for measurements it does not carry.
    fn roll_up(&mut self) {
        let indices: BTreeMap<_, _> = self
            .spans
            .iter()
            .enumerate()
            .map(|(index, span)| (span.id.clone(), index))
            .collect();
        let mut children = vec![Vec::new(); self.spans.len()];
        for (index, span) in self.spans.iter().enumerate() {
            if let Some(parent) = span.parent_id.as_ref().and_then(|id| indices.get(id)) {
                children[*parent].push(index);
            }
        }
        for field in [
            "gen_ai.usage.input_tokens",
            "gen_ai.usage.output_tokens",
            "thirdshift.usage.cache_read_tokens",
            "thirdshift.usage.cache_write_tokens",
            "thirdshift.cost.usd",
        ] {
            let mut totals = vec![None; self.spans.len()];
            let mut visited = BTreeSet::new();
            for index in 0..self.spans.len() {
                metric(
                    index,
                    field,
                    &self.spans,
                    &children,
                    &mut totals,
                    &mut visited,
                );
            }
            for (span, total) in self.spans.iter_mut().zip(totals) {
                if let Some(total) = total {
                    span.attributes.entry(field.into()).or_insert(total);
                }
            }
        }
    }
}

fn metric(
    index: usize,
    field: &str,
    spans: &[Span],
    children: &[Vec<usize>],
    totals: &mut [Option<Value>],
    visited: &mut BTreeSet<usize>,
) -> Option<Value> {
    if !visited.insert(index) {
        return totals[index].clone();
    }
    if let Some(value) = spans[index]
        .attributes
        .get(field)
        .filter(|value| !value.is_null())
    {
        totals[index] = Some(value.clone());
        return Some(value.clone());
    }
    let values: Vec<_> = children[index]
        .iter()
        .filter_map(|child| metric(*child, field, spans, children, totals, visited))
        .collect();
    let total = if field == "thirdshift.cost.usd" {
        values
            .iter()
            .filter_map(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str()?.parse::<f64>().ok())
            })
            .filter(|value| value.is_finite())
            .reduce(|a, b| a + b)
            .map(Value::from)
    } else {
        values
            .iter()
            .filter_map(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str()?.parse::<u64>().ok())
            })
            .reduce(u64::saturating_add)
            .map(Value::from)
    };
    totals[index] = total.clone();
    total
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
                title: command.record.issue.as_ref().and_then(|issue| {
                    issue
                        .title
                        .as_ref()
                        .filter(|title| !title.trim().is_empty())
                        .cloned()
                }),
                state: command
                    .record
                    .outcome
                    .clone()
                    .unwrap_or_else(|| command.state.into()),
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
        for node in ancestry::of(command, owner)? {
            tree.spans.extend(spans(&node)?);
        }
    }
    tree.roll_up();
    Ok(View { choices, tree })
}

/// Project one ancestry node, reading an attached Session's own spans.
fn spans(node: &Node) -> Result<Vec<Span>> {
    let command = node.command;
    let selector = Observation::Record(command).selector().unwrap_or_default();
    let id = node.id().map(hex).unwrap_or_default();
    let parent = node.parent.map(hex);
    Ok(match &node.fact {
        Fact::Command => vec![record_span(command, id, parent, SpanKind::Cmd, &selector)],
        Fact::Run => vec![record_span(command, id, parent, SpanKind::Run, &selector)],
        Fact::Stage(timing) => {
            let mut span = record_span(command, id, parent, SpanKind::Stage, &selector);
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
            let last = command
                .record
                .stages
                .last()
                .is_some_and(|stage| std::ptr::eq(stage, timing.stage));
            if last && command.record.cause.is_some() {
                span.error.clone_from(&command.record.cause);
                span.status = "error".into();
            }
            span.duration_ms = duration(span.start, span.end);
            vec![span]
        }
        Fact::Session(attachment) => {
            let mut template = record_span(command, id, parent, SpanKind::Sess, &selector);
            template.start = attachment
                .stage
                .as_ref()
                .and_then(|timing| timing.began)
                .map(|time| time.timestamp_millis() as f64)
                .or(template.start);
            template.input = None;
            template.conversation_url =
                Some(inspect::url("/log", &selector, Some(&attachment.path)));
            session::read(&attachment.path, &node.context, &template)?.unwrap_or_default()
        }
    })
}

fn record_span(
    command: &Command,
    id: String,
    parent_id: Option<String>,
    kind: SpanKind,
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
        kind,
        name: span_name(command),
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

fn span_name(command: &Command) -> String {
    let title = command
        .record
        .issue
        .as_ref()
        .and_then(|issue| issue.title.as_ref())
        .map(|title| format!(" · {title}"))
        .unwrap_or_default();
    if let Some(heading) = command.record.process_heading() {
        return format!("{heading}{title}");
    }
    format!("{} · {}{title}", command.record.kind, subject(command))
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
