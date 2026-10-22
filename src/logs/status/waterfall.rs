//! A Command tree on one time axis, derived from show facts and retained logs.
pub(in crate::logs) mod matching;
mod trace;

use anyhow::Result;
use chrono::{DateTime, FixedOffset};

use crate::logs::reading::Command;

type Time = DateTime<FixedOffset>;

#[derive(PartialEq)]
pub(in crate::logs) struct Call {
    pub(in crate::logs) identity: Option<(Option<String>, String)>,
    pub(in crate::logs) label: String,
    pub(in crate::logs) start: Option<Time>,
    pub(in crate::logs) end: Option<Time>,
}

#[derive(PartialEq)]
pub(in crate::logs) struct PlacedCall {
    pub(in crate::logs) identity: Option<(Option<String>, String)>,
    pub(in crate::logs) stage: Option<usize>,
    pub(in crate::logs) label: String,
    pub(in crate::logs) start: Option<Time>,
    pub(in crate::logs) end: Option<Time>,
}

/// The reader and the conversion report share stage assignment and inferred
/// end times, including whole-second legacy timestamps at stage boundaries.
pub(in crate::logs) fn place_calls(
    session: &std::path::Path,
    command: &Command,
    calls: &[Call],
    measured: bool,
    now: Time,
) -> Vec<PlacedCall> {
    let intervals: Vec<_> = command
        .record
        .stage_times((command.state == "live").then_some(now))
        .collect();
    let references = |stage: &crate::logs::command_record::Stage| {
        stage.session_log.as_deref().is_some_and(|path| {
            crate::logs::session::original(path) == crate::logs::session::original(session)
        })
    };
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let stage = if measured {
                call.start.and_then(|start| {
                    intervals.iter().position(|timing| {
                        timing.began.is_some_and(|began| start >= began)
                            && timing.ended.is_none_or(|end| start < end)
                    })
                })
            } else {
                call.start
                    .and_then(|start| {
                        intervals.iter().position(|timing| {
                            references(timing.stage)
                                && timing
                                    .began
                                    .is_some_and(|began| start.timestamp() >= began.timestamp())
                                && timing
                                    .ended
                                    .is_none_or(|end| start.timestamp() <= end.timestamp())
                        })
                    })
                    .or_else(|| {
                        call.start.and_then(|start| {
                            intervals.iter().position(|timing| {
                                timing
                                    .began
                                    .is_some_and(|began| start.timestamp() >= began.timestamp())
                                    && timing
                                        .ended
                                        .is_none_or(|end| start.timestamp() < end.timestamp())
                            })
                        })
                    })
            }
            .or_else(|| command.record.stages.iter().position(references));
            let boundary = stage.and_then(|stage| intervals[stage].ended);
            let next = calls[index + 1..].iter().find_map(|call| call.start);
            let end = if measured {
                call.end
                    .or_else(|| (command.state == "live").then_some(now))
            } else {
                match (next, boundary) {
                    (Some(next), Some(boundary)) => Some(next.min(boundary)),
                    (next, boundary) => next.or(boundary),
                }
            };
            PlacedCall {
                identity: call.identity.clone(),
                stage,
                label: call.label.clone(),
                start: call.start,
                end,
            }
        })
        .collect()
}

pub(in crate::logs) fn session_calls(
    session: &std::path::Path,
    command: &Command,
    relays: &[u64],
    log: &mut matching::Log,
) -> Result<(Vec<Call>, bool)> {
    Ok(match trace::calls(session)? {
        Some(calls) => (calls, true),
        None => (matching::calls(session, command, relays, log)?, false),
    })
}

fn time(value: &str) -> Option<Time> {
    DateTime::parse_from_rfc3339(value).ok()
}
