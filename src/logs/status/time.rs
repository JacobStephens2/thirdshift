//! Time views over the same complete observations as status --all --json.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::{DateTime, Local};

use crate::config::UserConfig;
use crate::logs::command_record::{Issue, StageName};
use crate::logs::reading::{Command, Entry, Observation, Snapshot};

pub(crate) struct Views {
    pub lanes: Vec<Vec<Run>>,
    pub grids: Vec<Grid>,
    pub start: String,
    pub end: String,
}

#[derive(Clone)]
pub(crate) struct Run {
    pub subject: String,
    pub title: String,
    pub state: String,
    pub selector: String,
    pub peek_url: String,
    pub log_url: String,
    pub issue_url: Option<String>,
    pub issue_label: String,
    pub stages: Vec<Stage>,
    repository: String,
    started: Option<i64>,
    ended: Option<i64>,
    stage_metadata: bool,
}

#[derive(Clone)]
pub(crate) struct Stage {
    pub name: String,
    pub running: bool,
    pub text: String,
    pub seconds: String,
    pub usual: String,
    pub left: f64,
    pub width: f64,
}

pub(crate) struct Grid {
    pub repository: String,
    pub runs: Vec<Run>,
    pub rows: Vec<GridRow>,
}

pub(crate) struct GridRow {
    pub name: String,
    pub usual: String,
    pub cells: Vec<Cell>,
}

pub(crate) struct Cell {
    pub text: String,
    pub running: bool,
}

fn timestamp(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp())
}

fn subject(kind: &str, repository: &str, issue: Option<&Issue>) -> String {
    let kind = crate::logs::display_name(kind);
    issue.map_or_else(
        || format!("{kind} on {repository}"),
        |issue| format!("{kind} · {repository}#{}", issue.number),
    )
}

fn issue_link(issue: Option<&Issue>) -> (Option<String>, String) {
    match issue {
        Some(issue) if issue.url.starts_with("https://") => {
            (Some(issue.url.clone()), format!("Issue #{}", issue.number))
        }
        _ => (None, String::new()),
    }
}

fn run(command: &Command, now: DateTime<Local>, cutoff: i64) -> Run {
    let record = &command.record;
    let selector = Observation::Record(command).selector().unwrap_or_default();
    let live_until = (command.state == "live").then_some(now.fixed_offset());
    let now = now.timestamp();
    let stages = record
        .stage_times(live_until)
        .enumerate()
        .map(|(index, timing)| {
            let stage = timing.stage;
            let began = timing.began.map(|time| time.timestamp());
            let end = timing.ended.map(|time| time.timestamp());
            let seconds = timing.seconds();
            let state = if index + 1 < record.stages.len() {
                "Complete"
            } else if command.state == "live" {
                "Running"
            } else if command.state == "died" {
                "Died"
            } else if record.outcome.as_deref() == Some("merged") {
                "Complete"
            } else {
                record.outcome.as_deref().unwrap_or("Unknown")
            };
            let duration = seconds
                .map(|seconds| super::age(seconds as i64))
                .unwrap_or_else(|| "time unknown".into());
            // Clip only the drawn interval; the text retains the full stage time.
            let (left, width) =
                began
                    .zip(end)
                    .filter(|_| seconds.is_some())
                    .map_or((0.0, 0.0), |(start, end)| {
                        let start = start.clamp(cutoff, now);
                        let end = end.clamp(cutoff, now);
                        let scale = 100.0 / (now - cutoff) as f64;
                        (
                            (start - cutoff) as f64 * scale,
                            (end - start).max(0) as f64 * scale,
                        )
                    });
            Stage {
                name: stage.name.clone(),
                running: command.state == "live" && index + 1 == record.stages.len(),
                text: format!("{state} · {duration}"),
                seconds: seconds
                    .map(|seconds| seconds.to_string())
                    .unwrap_or_default(),
                usual: if StageName::is_dispatch(&stage.name) {
                    "—".into()
                } else {
                    command
                        .usual_ranges
                        .get(&stage.name)
                        .map(|range| range.text())
                        .unwrap_or_else(|| "Not enough samples".into())
                },
                left,
                width,
            }
        })
        .collect();
    let (issue_url, issue_label) = issue_link(record.issue.as_ref());
    Run {
        subject: subject(&record.kind, &record.repository, record.issue.as_ref()),
        title: record
            .issue
            .as_ref()
            .and_then(|issue| issue.title.clone())
            .unwrap_or_else(|| record.name.clone()),
        state: if command.state == "ended" {
            record.outcome.clone().unwrap_or_else(|| "ended".into())
        } else {
            command.state.into()
        },
        peek_url: super::inspect::url("/peek", &selector, None),
        log_url: super::inspect::url("/log", &selector, None),
        issue_url,
        issue_label,
        selector,
        repository: record.repository.clone(),
        stages,
        started: timestamp(&record.started_at),
        ended: record.ended_at.as_deref().and_then(timestamp),
        stage_metadata: true,
    }
}

fn collect(command: &Command, now: DateTime<Local>, cutoff: i64, runs: &mut Vec<Run>) {
    runs.push(run(command, now, cutoff));
    for child in command.dispatched.iter().chain(&command.children) {
        collect(child, now, cutoff, runs);
    }
}

pub(crate) fn load(config: &UserConfig, hours: f64) -> Result<Views> {
    let now = Local::now();
    let cutoff = now - chrono::Duration::seconds((hours * 3600.0) as i64);
    let snapshot = Snapshot::load_with_silence(&config.logs_dir, now, config.silent_interval)?;
    let mut lanes = Vec::new();
    let mut repositories: BTreeMap<String, Vec<Run>> = BTreeMap::new();
    for entry in snapshot.entries() {
        let mut runs = Vec::new();
        match entry {
            Entry::Record(command) => collect(command, now, cutoff.timestamp(), &mut runs),
            Entry::History(command) => {
                let selector = Observation::History(command).selector().unwrap_or_default();
                let (issue_url, issue_label) = issue_link(command.issue.as_ref());
                runs.push(Run {
                    subject: subject(&command.kind, &command.repository, command.issue.as_ref()),
                    title: String::new(),
                    state: command
                        .outcome
                        .clone()
                        .unwrap_or_else(|| command.state.into()),
                    peek_url: super::inspect::url("/peek", &selector, None),
                    log_url: super::inspect::url("/log", &selector, None),
                    issue_url,
                    issue_label,
                    selector,
                    repository: command.repository.clone(),
                    stages: Vec::new(),
                    started: timestamp(&command.started_at),
                    ended: command.ended_at.as_deref().and_then(timestamp),
                    stage_metadata: false,
                });
            }
        }
        for run in &runs {
            repositories
                .entry(run.repository.clone())
                .or_default()
                .push(run.clone());
        }
        if runs.iter().any(|run| {
            run.state == "live"
                || run.started.is_some_and(|start| {
                    start <= now.timestamp() && run.ended.unwrap_or(start) > cutoff.timestamp()
                })
        }) {
            lanes.push(runs);
        }
    }
    lanes.sort_by_key(|runs| std::cmp::Reverse(runs.first().and_then(|run| run.started)));
    let mut ordered: Vec<_> = repositories.values().flatten().cloned().collect();
    ordered.sort_by_key(|run| {
        (
            run.state != "live",
            std::cmp::Reverse(run.ended.or(run.started)),
        )
    });
    // Keep global ordering, continuing a repository's table whenever the next
    // Run belongs elsewhere or three columns have filled the available width.
    let mut batches: Vec<Vec<Run>> = Vec::new();
    for run in ordered {
        if let Some(batch) = batches.last_mut()
            && batch.len() < 3
            && batch[0].repository == run.repository
        {
            batch.push(run);
        } else {
            batches.push(vec![run]);
        }
    }
    let mut grids = Vec::new();
    for batch in batches {
        let repository = &batch[0].repository;
        let runs = &repositories[repository];
        let names: BTreeSet<_> = runs
            .iter()
            .flat_map(|run| run.stages.iter().map(|stage| stage.name.clone()))
            .collect();
        let rows = names
            .iter()
            .map(|name| GridRow {
                name: name.clone(),
                usual: runs
                    .iter()
                    .flat_map(|run| &run.stages)
                    .find(|stage| &stage.name == name)
                    .map(|stage| stage.usual.clone())
                    .unwrap_or_default(),
                cells: batch
                    .iter()
                    .map(|run| {
                        let stages: Vec<_> = run
                            .stages
                            .iter()
                            .filter(|stage| &stage.name == name)
                            .collect();
                        let text = if !run.stage_metadata {
                            "Unknown · no stage times retained".into()
                        } else if stages.is_empty() {
                            "Not reached · —".into()
                        } else {
                            stages
                                .iter()
                                .map(|stage| stage.text.as_str())
                                .collect::<Vec<_>>()
                                .join("; ")
                        };
                        Cell {
                            text,
                            running: stages.iter().any(|stage| stage.running),
                        }
                    })
                    .collect(),
            })
            .collect();
        grids.push(Grid {
            repository: repository.clone(),
            runs: batch,
            rows,
        });
    }
    Ok(Views {
        lanes,
        grids,
        start: cutoff.format("%Y-%m-%d %H:%M %Z").to_string(),
        end: now.format("%Y-%m-%d %H:%M %Z").to_string(),
    })
}
