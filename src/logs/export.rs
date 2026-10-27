//! A Command's complete local Trace, with record-derived ancestry and times.
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use chrono::{DateTime, Local};

use super::reading::{Command, Snapshot};
use super::trace::Context;
use super::trace::ancestry::{self, Fact, Node};
use crate::config::UserConfig;
use crate::otlp_jsonl::{
    AnyValue, KeyValue, Logs, Reader, Request, Resource, ResourceSpans, Scope, ScopeSpans, Span,
    SpanId, Spans, Writer,
};

pub fn export(input: &str, directory: &Path) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::for_user(&config, Local::now())?;
    let selection = snapshot.resolve(input)?;
    let command = selection
        .owner
        .context("export requires a Command record")?;
    fs::create_dir_all(directory)?;
    let spans_path = directory.join("spans.jsonl");
    let logs_path = directory.join("logs.jsonl");
    ensure!(
        !spans_path.exists() && !logs_path.exists(),
        "export files already exist in {}",
        directory.display()
    );
    let mut spans = tempfile::NamedTempFile::new_in(directory)?;
    let mut logs = tempfile::NamedTempFile::new_in(directory)?;

    // Whole export starts at the owner, so a Ticket keeps its siblings.
    let nodes = ancestry::of(command, command)?;
    let context = nodes[0].context.clone();
    let mut validation = Validation::default();
    let mut recorded = Vec::new();
    for node in &nodes {
        recorded.extend(record_span(node)?);
    }
    // Attachments select Session files; their requests keep recorded parents.
    let mut sessions: BTreeSet<_> = nodes
        .iter()
        .filter_map(|node| match &node.fact {
            Fact::Session(attachment) => Some(attachment.original.clone()),
            _ => None,
        })
        .collect();
    for node in &nodes {
        if matches!(node.fact, Fact::Run)
            && let Some(experiment) = &node.command.record.experiment
        {
            sessions.extend(experiment.trace_sessions(&context)?);
        }
    }
    for span in &recorded {
        validation.span(span, &context)?;
    }
    Writer::new(&mut spans).write(&Spans {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![
                    attribute("service.name", "thirdshift"),
                    attribute("service.version", env!("CARGO_PKG_VERSION")),
                    attribute("thirdshift.repository", &command.record.repository),
                    attribute("thirdshift.command.identity", &context.identity),
                ],
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(Scope {
                    name: "thirdshift.logs.export".into(),
                    ..Default::default()
                }),
                spans: recorded,
                ..Default::default()
            }],
            ..Default::default()
        }],
    })?;
    for session in sessions {
        let session_spans = session.with_extension("spans.jsonl");
        let session_logs = session.with_extension("logs.jsonl");
        // Legacy Sessions have no OTLP trace to export.
        if !session_spans.exists() && !session_logs.exists() {
            continue;
        }
        copy_requests::<Spans>(&session_spans, &mut spans, |request| {
            for span in request
                .resource_spans
                .iter()
                .flat_map(|resource| &resource.scope_spans)
                .flat_map(|scope| &scope.spans)
            {
                validation.span(span, &context)?;
            }
            Ok(())
        })?;
        copy_requests::<Logs>(&session_logs, &mut logs, |request| {
            for log in request
                .resource_logs
                .iter()
                .flat_map(|resource| &resource.scope_logs)
                .flat_map(|scope| &scope.log_records)
            {
                ensure!(
                    log.trace_id == Some(context.trace),
                    "Session trace belongs to another Trace"
                );
                validation.references.extend(log.span_id);
            }
            Ok(())
        })?;
    }
    ensure!(
        validation
            .references
            .iter()
            .all(|id| validation.ids.contains(&id.0)),
        "incomplete Trace: a parent or log span is missing; wait for the Command to finish"
    );
    spans.persist_noclobber(&spans_path)?;
    logs.persist_noclobber(&logs_path)?;
    println!(
        "Exported Trace to {} and {}",
        spans_path.display(),
        logs_path.display()
    );
    Ok(())
}

/// Project a record node with OTLP times; Sessions export their own spans.
fn record_span(node: &Node) -> Result<Option<Span>> {
    let command = node.command;
    let (name, start, end, attributes) = match &node.fact {
        Fact::Command => (
            "Command".into(),
            timestamp(&command.record.started_at)?,
            end_time(command)?,
            Vec::new(),
        ),
        Fact::Run => (
            format!("{} {}", command.record.kind, command.record.name),
            timestamp(&command.record.started_at)?,
            end_time(command)?,
            vec![attribute("thirdshift.kind", &command.record.kind)],
        ),
        Fact::Stage(time) => (
            time.stage.name.clone(),
            timestamp(&time.stage.began_at)?,
            time.ended
                .map(|time| timestamp(&time.to_rfc3339()))
                .transpose()?
                .unwrap_or_default(),
            Vec::new(),
        ),
        Fact::Session(_) => return Ok(None),
    };
    Ok(Some(Span {
        trace_id: Some(node.context.trace),
        span_id: node.id(),
        parent_span_id: node.parent,
        name,
        kind: 1,
        start_time_unix_nano: start,
        end_time_unix_nano: end,
        attributes,
        ..Default::default()
    }))
}

fn end_time(command: &Command) -> Result<u64> {
    command
        .record
        .ended_at
        .as_deref()
        .map(timestamp)
        .transpose()
        .map(Option::unwrap_or_default)
}

fn timestamp(value: &str) -> Result<u64> {
    let time = DateTime::parse_from_rfc3339(value)?;
    time.timestamp_nanos_opt()
        .and_then(|nanos| nanos.try_into().ok())
        .context("Command record time is outside the OTLP nanosecond range")
}

fn attribute(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.into(),
        value: Some(AnyValue::String(value.into())),
    }
}

#[derive(Default)]
struct Validation {
    ids: HashSet<[u8; 8]>,
    references: Vec<SpanId>,
}

impl Validation {
    fn span(&mut self, span: &Span, context: &Context) -> Result<()> {
        ensure!(
            span.trace_id == Some(context.trace),
            "Session span belongs to another Trace"
        );
        let id = span.span_id.context("span has no ID")?;
        ensure!(self.ids.insert(id.0), "duplicate span ID in Trace");
        self.references.extend(span.parent_span_id);
        Ok(())
    }
}

/// Validate each request but retain its exact bytes, including unknown fields.
fn copy_requests<R: Request>(
    path: &Path,
    output: &mut impl Write,
    mut check: impl FnMut(&R) -> Result<()>,
) -> Result<()> {
    let file = File::open(path)
        .with_context(|| format!("could not open Session trace {}", path.display()))?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        let request = Reader::<_, R>::new(line.as_bytes())
            .next()
            .context("empty Session trace line")?
            .with_context(|| format!("could not decode Session trace {}", path.display()))?;
        check(&request)?;
        writeln!(output, "{line}")?;
    }
    Ok(())
}
