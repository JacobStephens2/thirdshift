//! Convert runner-owned Session records into the Trial's retained traces.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

use crate::harness::interpretation::event::{ErrorKind, Event, Usage, UsageScope};
use crate::harness::interpretation::limits::Limit;
use crate::harness::{Choice, Harness};
use crate::logs::trace::Context;
use crate::session::trace::Trace;

mod confined;
mod transcript;

#[derive(Default)]
pub(crate) struct Import {
    pub sessions: usize,
    pub harness_sessions: usize,
    pub web_search: bool,
    pub usage_limit: Option<Limit>,
}

pub(crate) fn import(
    output_dir: &Path,
    destination: &Path,
    repository: &Path,
    context: &Context,
) -> Result<Import> {
    let directory = confined::Directory::open(output_dir)?;
    let Some(bytes) = directory.read("sessions.json")? else {
        return Ok(Import::default());
    };
    let sessions: Vec<Session> =
        serde_json::from_slice(&bytes).context("invalid Custom runner sessions.json")?;
    fs::create_dir_all(destination)?;
    let mut imported = Import::default();
    let mut files = BTreeSet::new();
    for (index, session) in sessions.into_iter().enumerate() {
        let started = nanos(&session.started_at)?;
        let ended = nanos(&session.ended_at)?;
        ensure!(
            ended >= started,
            "Custom runner Session ended before it started"
        );
        let filename = match &session.source {
            Source::Harness { stream_file, .. } => stream_file,
            Source::Sdk {
                transcript_file, ..
            } => transcript_file,
        };
        ensure!(
            files.insert(filename.clone()),
            "Custom runner Session file is listed more than once"
        );
        let bytes = directory
            .read(filename)?
            .context("missing Custom runner Session file")?;
        let path = destination.join(format!("custom-import-{}.jsonl", index + 1));
        let harness = match &session.source {
            Source::Harness { harness, .. } => Harness::named(harness)
                .with_context(|| format!("unsupported Custom runner Harness: {harness}"))?,
            Source::Sdk { .. } => Harness::Codex,
        };
        let name = match &session.source {
            Source::Harness { .. } => harness.name(),
            Source::Sdk { .. } => "sdk",
        };
        let mut command = Command::new(name);
        command.current_dir(repository);
        let mut trace = Trace::create(
            &path,
            "custom",
            &Choice {
                harness,
                ..Default::default()
            },
            "",
            context.clone(),
            &command,
            None,
        )?
        .with_started_at(started)
        .with_harness(name)
        .with_trial_role(Some(&session.role))?;
        let error = match session.source {
            Source::Harness { .. } => {
                let error = raw(
                    &bytes,
                    &mut trace,
                    harness,
                    repository,
                    started,
                    ended,
                    &mut imported,
                )?;
                imported.harness_sessions += 1;
                error
            }
            Source::Sdk { models, .. } => {
                let transcript =
                    String::from_utf8(bytes).context("SDK transcript must be UTF-8")?;
                imported.web_search |= sdk(&transcript, models, &mut trace, started, ended)?;
                None
            }
        };
        trace.finish_at(error.as_deref(), ended)?;
        imported.sessions += 1;
    }
    Ok(imported)
}

fn sdk(
    transcript: &str,
    models: BTreeMap<String, Tokens>,
    trace: &mut Trace,
    started: u64,
    ended: u64,
) -> Result<bool> {
    let web_search = transcript::record(transcript, trace, started)?;
    for (model, tokens) in models {
        let input = tokens
            .uncached_input_tokens
            .checked_add(tokens.cache_read_tokens)
            .and_then(|input| input.checked_add(tokens.cache_write_tokens))
            .context("Custom runner input token count overflow")?;
        let event = Event::Usage {
            scope: UsageScope::Model,
            model: Some(model),
            usage: Usage {
                input: Some(input),
                output: Some(tokens.output_tokens),
                cache_read: Some(tokens.cache_read_tokens),
                cache_write: Some(tokens.cache_write_tokens),
            },
            cost_usd: None,
        };
        trace.record_at(&serde_json::to_vec(&event)?, &[event], ended)?;
    }
    Ok(web_search)
}

fn raw(
    bytes: &[u8],
    trace: &mut Trace,
    harness: Harness,
    repository: &Path,
    started: u64,
    ended: u64,
    imported: &mut Import,
) -> Result<Option<String>> {
    let mut interpretation = harness
        .adapter()
        .interpretation(repository, "")
        .without_retained();
    let mut seen = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let mut events = interpretation.decode(&String::from_utf8_lossy(line)).events;
        strip_costs(&mut events);
        imported.web_search |= web_search(&events);
        imported.usage_limit = imported
            .usage_limit
            .take()
            .or_else(|| interpretation.plan_limit().cloned())
            .or_else(|| quota_error(harness, &events));
        trace.record_at(line, &events, started)?;
        seen.extend(events);
    }
    let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(0)));
    if let Some(report) = completion.report {
        let mut events = report.events;
        strip_costs(&mut events);
        imported.web_search |= web_search(&events);
        imported.usage_limit = imported
            .usage_limit
            .take()
            .or_else(|| quota_error(harness, &events));
        for event in events {
            if !seen.contains(&event) {
                trace.record_at(&serde_json::to_vec(&event)?, &[event], ended)?;
            }
        }
    }
    Ok(completion.outcome.err().map(|error| format!("{error:#}")))
}

fn strip_costs(events: &mut [Event]) {
    for event in events {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only usage facts can carry a runner-reported cost"
        )]
        match event {
            Event::Usage { cost_usd, .. } | Event::ModelCallEnded { cost_usd, .. } => {
                *cost_usd = None
            }
            _ => {}
        }
    }
}

fn quota_error(harness: Harness, events: &[Event]) -> Option<Limit> {
    events
        .iter()
        .any(|event| {
            matches!(
                event,
                Event::Error {
                    kind: ErrorKind::UsageLimit,
                    ..
                }
            )
        })
        .then(|| {
            Limit {
                harness: Some(harness.name().into()),
                ..Default::default()
            }
            .accepted()
        })
}

#[derive(Deserialize)]
struct Session {
    role: String,
    started_at: String,
    ended_at: String,
    #[serde(flatten)]
    source: Source,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Source {
    Harness {
        harness: String,
        stream_file: String,
    },
    Sdk {
        transcript_file: String,
        models: BTreeMap<String, Tokens>,
    },
}

#[derive(Deserialize)]
struct Tokens {
    uncached_input_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    output_tokens: u64,
}

fn nanos(text: &str) -> Result<u64> {
    chrono::DateTime::parse_from_rfc3339(text)?
        .timestamp_nanos_opt()
        .and_then(|time| u64::try_from(time).ok())
        .context("Custom runner Session timestamp is out of range")
}

pub(crate) fn web_search(events: &[Event]) -> bool {
    events.iter().any(|event| {
        let (tool, input) = match event {
            Event::ToolCallStarted { tool, input, .. }
            | Event::ToolCallEnded { tool, input, .. } => (tool, input),
            Event::ModelCallStarted { .. }
            | Event::ModelCallEnded { .. }
            | Event::TurnStarted { .. }
            | Event::TurnEnded { .. }
            | Event::Message { .. }
            | Event::Usage { .. }
            | Event::Error { .. }
            | Event::WorkEnded { .. }
            | Event::SubagentStarted { .. }
            | Event::SubagentEnded { .. } => return false,
        };
        let normalized = tool.to_ascii_lowercase().replace(['_', '-', '.'], "");
        normalized.ends_with("websearch")
            || normalized.ends_with("websearchpreview")
            || normalized.ends_with("websearchcall")
            || (normalized == "webrun" && input.get("search_query").is_some())
    })
}

#[cfg(test)]
mod tests;
