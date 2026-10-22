//! Replay the Harness's own summaries, consuming exact scoped log lines once.
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveTime, TimeZone};
use serde_json::Value;

use super::{Call, Time, time};
use crate::harness::Harness;
use crate::logs::owner;
use crate::logs::reading::Command;

struct Line {
    time: Time,
    relays: Vec<u64>,
    message: String,
    used: bool,
}
pub(in crate::logs) struct Log {
    lines: Vec<Line>,
}

fn read(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
    }
}

impl Log {
    pub(in crate::logs) fn read(path: &Path, start: Time) -> Result<Self> {
        let mut lines = Vec::new();
        let mut previous = None;
        let mut date = start.date_naive();
        for raw in read(path)?.lines() {
            let Some(rest) = raw.strip_prefix("thirdshift: ") else {
                continue;
            };
            let Some((stamp, mut message)) = rest.split_once(' ') else {
                continue;
            };
            let Ok(clock) = NaiveTime::parse_from_str(stamp, "%H:%M:%S") else {
                continue;
            };
            // Relayed lines can arrive slightly out of order; only a large
            // backwards jump is a midnight, not a scheduling delay.
            if previous.is_some_and(|previous: NaiveTime| {
                previous.signed_duration_since(clock) > Duration::hours(12)
            }) {
                date += Duration::days(1);
            } else if previous.is_none()
                && clock.signed_duration_since(start.time()) > Duration::hours(12)
            {
                date -= Duration::days(1);
            }
            previous = Some(clock);
            let Some(time) = start
                .offset()
                .from_local_datetime(&date.and_time(clock))
                .single()
            else {
                continue;
            };
            let mut relays = Vec::new();
            while let Some((prefix, rest)) = message.split_once(": ") {
                let Some(number) = prefix.strip_prefix('#').and_then(|n| n.parse().ok()) else {
                    break;
                };
                relays.push(number);
                message = rest;
            }
            lines.push(Line {
                time,
                relays,
                message: message.into(),
                used: false,
            });
        }
        Ok(Self { lines })
    }

    pub(in crate::logs) fn last_time(&self) -> Option<Time> {
        self.lines.iter().map(|line| line.time).max()
    }

    pub(in crate::logs) fn estimate(
        &mut self,
        label: &str,
        file: Option<&str>,
        kind: &str,
        relays: &[u64],
        bounds: (Option<Time>, Option<Time>),
        cursor: &mut usize,
    ) -> Option<Time> {
        let message = format!("{kind}: {label}");
        let (began, ended) = bounds;
        let matched = self
            .lines
            .iter_mut()
            .enumerate()
            .skip(*cursor)
            .find(|(_, line)| {
                !line.used
                    && line.relays == relays
                    && (line.message == message
                        || file.is_some_and(|path| {
                            let Some(summary) = line.message.strip_prefix(&format!("{kind}: "))
                            else {
                                return false;
                            };
                            let Some((action, detail)) = summary.split_once(' ') else {
                                return false;
                            };
                            if label.split_once(' ').map(|(action, _)| action) != Some(action) {
                                return false;
                            }
                            if let Some(prefix) = detail.strip_suffix('…') {
                                path.trim_start_matches('/').starts_with(prefix)
                                    || path.contains(&format!("/{prefix}"))
                            } else {
                                path == detail || path.ends_with(&format!("/{detail}"))
                            }
                        }))
                    && began.is_none_or(|began| line.time.timestamp() >= began.timestamp())
                    && ended.is_none_or(|ended| line.time <= ended)
            });
        matched.map(|(index, line)| {
            *cursor = index + 1;
            line.used = true;
            line.time
        })
    }
}

pub(super) fn calls(
    session: &Path,
    command: &Command,
    relays: &[u64],
    log: &mut Log,
) -> Result<Vec<Call>> {
    let text = read(&crate::logs::session::path(session))?;
    let Some(harness) = Harness::named(&command.record.harness) else {
        return Ok(Vec::new());
    };
    let mut stream = harness.adapter().interpretation(Path::new(""), "");
    let kind = owner::decode(session)
        .and_then(|decoded| decoded.session_kind)
        .or_else(|| {
            session
                .file_stem()?
                .to_str()?
                .rsplit_once('-')
                .map(|(_, kind)| kind)
        })
        .unwrap_or("");
    let began = time(&command.record.started_at);
    let ended = command.record.ended_at.as_deref().and_then(time);
    let mut calls = Vec::new();
    let mut cursor = 0;
    for raw in crate::logs::session::events(&text) {
        let labels = stream.condense(&raw);
        let Ok(event) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let is_tool = (event["type"] == "assistant"
            && event["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "tool_use")))
            || (event["type"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("item."))
                && matches!(
                    event["item"]["type"].as_str(),
                    Some(
                        "command_execution"
                            | "file_change"
                            | "mcp_tool_call"
                            | "web_search"
                            | "collab_tool_call"
                    )
                ));
        if !is_tool {
            continue;
        }
        for (label_index, label) in labels.into_iter().enumerate() {
            // Codex does not retain its cwd. File changes retain absolute
            // paths, while progress prints paths relative to that cwd.
            let file = event["item"]["changes"][label_index]["path"].as_str();
            let start = log.estimate(&label, file, kind, relays, (began, ended), &mut cursor);
            calls.push(Call {
                label,
                start,
                end: None,
            });
        }
    }
    Ok(calls)
}
