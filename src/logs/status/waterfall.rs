//! A Command tree on one time axis, derived from show facts and retained logs.
mod matching;
mod trace;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};

use crate::config::UserConfig;
use crate::logs::reading::{Command, Entry, Observation, Snapshot};

type Time = DateTime<FixedOffset>;

pub(crate) struct View {
    pub choices: Vec<Choice>,
    pub runs: Vec<Run>,
    pub start: String,
    pub end: String,
}

pub(crate) struct Choice {
    pub subject: String,
    pub state: String,
    pub url: String,
    pub selected: bool,
}

pub(crate) struct Run {
    pub subject: String,
    pub kind: String,
    pub title: String,
    pub state: String,
    pub selector: String,
    pub peek_url: String,
    pub log_url: String,
    pub depth: usize,
    pub span: Span,
    pub stages: Vec<Stage>,
}

pub(crate) struct Stage {
    pub name: String,
    pub span: Span,
    pub tools: Vec<Tool>,
}

pub(crate) struct Tool {
    pub label: String,
    pub span: Span,
    pub url: String,
}

struct Call {
    label: String,
    start: Option<Time>,
    end: Option<Time>,
}

pub(crate) struct Span {
    pub start: String,
    pub seconds: String,
    pub text: String,
    pub left: f64,
    pub width: f64,
}

struct Axis {
    start: Time,
    end: Time,
}

impl Axis {
    fn span(&self, start: Option<Time>, end: Option<Time>) -> Span {
        let seconds = start.zip(end).and_then(|(start, end)| {
            let seconds = (end - start).as_seconds_f64();
            (seconds >= 0.0).then_some(seconds)
        });
        let scale = 100.0 / (self.end - self.start).as_seconds_f64().max(f64::EPSILON);
        Span {
            start: start.map(|time| time.to_rfc3339()).unwrap_or_default(),
            seconds: seconds.map(|n| n.to_string()).unwrap_or_default(),
            text: seconds
                .map(|n| crate::progress::duration(std::time::Duration::from_secs_f64(n)))
                .unwrap_or_else(|| "Time unknown".into()),
            left: start
                .map(|time| (time - self.start).as_seconds_f64() * scale)
                .unwrap_or(0.0)
                .clamp(0.0, 100.0),
            width: start
                .zip(end)
                .filter(|_| seconds.is_some())
                .map(|(start, end)| {
                    (end.min(self.end) - start.max(self.start))
                        .as_seconds_f64()
                        .max(0.0)
                        * scale
                })
                .unwrap_or(0.0)
                .clamp(0.0, 100.0),
        }
    }
}

fn time(value: &str) -> Option<Time> {
    DateTime::parse_from_rfc3339(value).ok()
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
    command
        .record
        .outcome
        .clone()
        .unwrap_or_else(|| command.state.into())
}

fn relay_path(command: &Command, selected: &Command, path: &mut Vec<u64>) -> bool {
    if std::ptr::eq(command, selected) {
        return true;
    }
    for dispatch in &command.dispatched {
        if relay_path(dispatch, selected, path) {
            return true;
        }
    }
    for child in &command.children {
        let length = path.len();
        if let Some(issue) = &child.record.issue {
            path.push(issue.number);
        }
        if relay_path(child, selected, path) {
            return true;
        }
        path.truncate(length);
    }
    false
}

fn collect(
    command: &Command,
    axis: &Axis,
    now: Time,
    depth: usize,
    relays: &[u64],
    log: &mut matching::Log,
    runs: &mut Vec<Run>,
) -> Result<()> {
    let selector = Observation::Record(command).selector().unwrap_or_default();
    let record = &command.record;
    let end = record
        .ended_at
        .as_deref()
        .and_then(time)
        .or_else(|| (command.state == "live").then_some(now));
    let intervals: Vec<_> = record
        .stage_times((command.state == "live").then_some(now))
        .collect();
    let mut stages: Vec<_> = intervals
        .iter()
        .map(|timing| Stage {
            name: timing.stage.name.clone(),
            span: axis.span(timing.began, timing.ended),
            tools: Vec::new(),
        })
        .collect();
    for session in Observation::Record(command).sessions()? {
        let traced = trace::calls(&session)?;
        let measured = traced.is_some();
        let calls = match traced {
            Some(calls) => calls,
            None => matching::calls(&session, command, relays, log)?,
        };
        for (index, call) in calls.iter().enumerate() {
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
                        // A log timestamp denotes its whole second. Prefer an
                        // explicit Session reference among compatible intervals,
                        // especially when a transition retains fractional time.
                        intervals.iter().position(|timing| {
                            timing.stage.session_log.as_ref() == Some(&session)
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
            .or_else(|| {
                record
                    .stages
                    .iter()
                    .position(|stage| stage.session_log.as_ref() == Some(&session))
            });
            let stage_index = stage.unwrap_or_else(|| {
                if stages.len() == intervals.len() {
                    stages.push(Stage {
                        name: "Session log · stage unknown".into(),
                        span: axis.span(None, None),
                        tools: Vec::new(),
                    });
                }
                intervals.len()
            });
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
            stages[stage_index].tools.push(Tool {
                label: call.label.clone(),
                span: axis.span(call.start, end),
                url: super::inspect::url("/log", &selector, Some(&session)),
            });
        }
    }
    runs.push(Run {
        subject: subject(command),
        kind: record.kind.clone(),
        title: record
            .issue
            .as_ref()
            .and_then(|issue| issue.title.clone())
            .unwrap_or_default(),
        state: state(command),
        peek_url: super::inspect::url("/peek", &selector, None),
        log_url: super::inspect::url("/log", &selector, None),
        selector,
        depth,
        span: axis.span(time(&record.started_at), end),
        stages,
    });
    for dispatch in &command.dispatched {
        collect(dispatch, axis, now, depth + 1, relays, log, runs)?;
    }
    for child in &command.children {
        let mut relays = relays.to_vec();
        if let Some(issue) = &child.record.issue {
            relays.push(issue.number);
        }
        collect(child, axis, now, depth + 1, &relays, log, runs)?;
    }
    Ok(())
}

fn last_trace_time(command: &Command) -> Result<Option<Time>> {
    let mut last = None;
    for session in Observation::Record(command).sessions()? {
        if let Some(calls) = trace::calls(&session)? {
            last = last.max(
                calls
                    .iter()
                    .filter_map(|call| call.end.or(call.start))
                    .max(),
            );
        }
    }
    for child in command.dispatched.iter().chain(&command.children) {
        last = last.max(last_trace_time(child)?);
    }
    Ok(last)
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
    let selected = if input.is_empty() {
        commands.first().copied()
    } else {
        match snapshot.resolve(input)?.observation {
            Observation::Record(command) => Some(command),
            _ => None,
        }
    };
    let selected_id = selected.and_then(|command| Observation::Record(command).selector());
    let choices = commands
        .iter()
        .map(|command| {
            let selector = Observation::Record(command).selector().unwrap_or_default();
            Choice {
                subject: subject(command),
                state: state(command),
                selected: Some(&selector) == selected_id.as_ref(),
                url: super::inspect::url("/waterfall", &selector, None),
            }
        })
        .collect();
    let mut view = View {
        choices,
        runs: Vec::new(),
        start: String::new(),
        end: String::new(),
    };
    if let Some(command) = selected {
        let start = time(&command.record.started_at).unwrap_or(now.fixed_offset());
        let mut relays = Vec::new();
        let owner = commands
            .iter()
            .find(|root| relay_path(root, command, &mut relays))
            .copied()
            .unwrap_or(command);
        let mut log = matching::Log::read(
            &owner.command_log,
            time(&owner.record.started_at).unwrap_or(start),
        )?;
        let known_end = command
            .record
            .ended_at
            .as_deref()
            .and_then(time)
            .or_else(|| (command.state == "live").then_some(now.fixed_offset()));
        let end = match known_end {
            Some(end) => end,
            // A dead owner could not record its end or print another progress
            // line, but its Session traces still retain completed tool spans.
            None => log
                .last_time()
                .max(last_trace_time(command)?)
                .unwrap_or(start),
        }
        .max(start);
        let axis = Axis { start, end };
        view.start = start.to_rfc3339();
        view.end = end.to_rfc3339();
        collect(
            command,
            &axis,
            now.fixed_offset(),
            0,
            &relays,
            &mut log,
            &mut view.runs,
        )?;
    }
    Ok(view)
}
