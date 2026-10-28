//! Time views over the same complete observations as status --all --json.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};

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
    pub context: String,
    pub group_start: bool,
    repository: String,
    started: Option<DateTime<FixedOffset>>,
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
    pub groups: Vec<Group>,
    pub runs: Vec<Run>,
    pub rows: Vec<GridRow>,
}

pub(crate) struct GridRow {
    pub name: String,
    pub cells: Vec<Cell>,
}

pub(crate) struct Cell {
    pub text: String,
    pub running: bool,
    pub usual: String,
    pub group_start: bool,
}

#[derive(Clone)]
pub(crate) struct Group {
    pub identity: String,
    pub subject: String,
    pub started: String,
    pub runs: Vec<Run>,
}

impl Group {
    fn new(run: Run, started: &str) -> Self {
        Self {
            identity: run.selector.clone(),
            subject: run.subject.clone(),
            started: if run.started.is_some() {
                started.into()
            } else {
                "unknown".into()
            },
            runs: vec![run],
        }
    }
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

fn account_subject(subject: String, account: Option<&str>) -> String {
    match account {
        Some(account) => format!("[{account}] {subject}"),
        None => subject,
    }
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
        subject: account_subject(
            record.process_heading().unwrap_or_else(|| {
                subject(&record.kind, &record.repository, record.issue.as_ref())
            }),
            command.account.as_deref(),
        ),
        title: record
            .issue
            .as_ref()
            .and_then(|issue| issue.title.clone())
            .or_else(|| record.process_label().map(str::to_string))
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
        context: String::new(),
        group_start: true,
        stages,
        started: DateTime::parse_from_rfc3339(&record.started_at).ok(),
        ended: record.ended_at.as_deref().and_then(timestamp),
        stage_metadata: true,
    }
}

// Membership follows exact recorded ancestry, never an issue number shared by attempts.
fn collect(
    command: &Command,
    now: DateTime<Local>,
    cutoff: i64,
    spec: Option<usize>,
    context: &str,
    groups: &mut Vec<Group>,
    runs: &mut Vec<Run>,
) {
    let mut current = run(command, now, cutoff);
    runs.push(current.clone());
    current.context = context.into();
    let spec = if command.record.kind == "Run"
        && let Some(index) = spec
    {
        current.context = format!(
            "Ticket of {} · Started {}",
            groups[index].subject, groups[index].started
        );
        current.group_start = false;
        groups[index].runs.push(current);
        spec
    } else {
        let index = groups.len();
        groups.push(Group::new(current, &command.record.started_at));
        (command.record.kind == "Spec run").then_some(index)
    };
    let parent = account_subject(
        subject(
            &command.record.kind,
            &command.record.repository,
            command.record.issue.as_ref(),
        ),
        command.account.as_deref(),
    );
    for dispatched in &command.dispatched {
        collect(
            dispatched,
            now,
            cutoff,
            spec,
            &format!("Dispatched by {parent}"),
            groups,
            runs,
        );
    }
    for child in &command.children {
        collect(
            child,
            now,
            cutoff,
            spec,
            &format!("Child of {parent}"),
            groups,
            runs,
        );
    }
}

fn grid(repository: String, groups: Vec<Group>) -> Grid {
    let runs: Vec<_> = groups.iter().flat_map(|group| group.runs.clone()).collect();
    let names: BTreeSet<_> = runs
        .iter()
        .flat_map(|run| {
            run.stages
                .iter()
                .map(|stage| (StageName::order(&stage.name), stage.name.clone()))
        })
        .collect();
    let rows = names
        .into_iter()
        .map(|(_, name)| {
            let cells = runs
                .iter()
                .map(|run| {
                    let stages: Vec<_> = run
                        .stages
                        .iter()
                        .filter(|stage| stage.name == name)
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
                    let usual = if StageName::is_dispatch(&name) {
                        "—".into()
                    } else {
                        runs.iter()
                            .filter(|sample| sample.repository == run.repository)
                            .flat_map(|sample| &sample.stages)
                            .find(|stage| stage.name == name)
                            .map(|stage| stage.usual.clone())
                            .unwrap_or_else(|| "Not enough samples".into())
                    };
                    Cell {
                        text,
                        running: stages.iter().any(|stage| stage.running),
                        usual,
                        group_start: run.group_start,
                    }
                })
                .collect();
            GridRow { name, cells }
        })
        .collect();
    Grid {
        repository,
        groups,
        runs,
        rows,
    }
}

pub(crate) fn load(config: &UserConfig, hours: f64, by_repository: bool) -> Result<Views> {
    let now = Local::now();
    let cutoff = now - chrono::Duration::seconds((hours * 3600.0) as i64);
    let snapshot = Snapshot::for_user(config, now)?;
    let mut lanes = Vec::new();
    let mut groups = Vec::new();
    for entry in snapshot.entries() {
        let mut runs = Vec::new();
        match entry {
            Entry::Record(command) => {
                collect(
                    command,
                    now,
                    cutoff.timestamp(),
                    None,
                    "",
                    &mut groups,
                    &mut runs,
                );
            }
            Entry::History(command) => {
                let selector = Observation::History(command).selector().unwrap_or_default();
                let (issue_url, issue_label) = issue_link(command.issue.as_ref());
                runs.push(Run {
                    subject: account_subject(
                        subject(&command.kind, &command.repository, command.issue.as_ref()),
                        command.account.as_deref(),
                    ),
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
                    context: String::new(),
                    group_start: true,
                    stages: Vec::new(),
                    started: DateTime::parse_from_rfc3339(&command.started_at).ok(),
                    ended: command.ended_at.as_deref().and_then(timestamp),
                    stage_metadata: false,
                });
                groups.push(Group::new(runs[0].clone(), &command.started_at));
            }
        }
        if runs.iter().any(|run| {
            run.state == "live"
                || run.started.is_some_and(|start| {
                    let start = start.timestamp();
                    start <= now.timestamp() && run.ended.unwrap_or(start) > cutoff.timestamp()
                })
        }) {
            lanes.push(runs);
        }
    }
    lanes.sort_by_key(|runs| {
        std::cmp::Reverse(
            runs.first()
                .and_then(|run| run.started.map(|start| start.timestamp())),
        )
    });
    groups.sort_by_key(|group| {
        (
            std::cmp::Reverse(group.runs[0].started),
            group.identity.clone(),
        )
    });
    for group in &mut groups {
        group.runs[1..].sort_by_key(|run| (std::cmp::Reverse(run.started), run.selector.clone()));
    }
    let grids = if by_repository {
        let mut repositories: BTreeMap<String, Vec<Group>> = BTreeMap::new();
        for group in groups {
            repositories
                .entry(group.runs[0].repository.clone())
                .or_default()
                .push(group);
        }
        repositories
            .into_iter()
            .map(|(repository, groups)| grid(repository, groups))
            .collect()
    } else if groups.is_empty() {
        Vec::new()
    } else {
        vec![grid("All runs".into(), groups)]
    };
    Ok(Views {
        lanes,
        grids,
        start: cutoff.format("%Y-%m-%d %H:%M %Z").to_string(),
        end: now.format("%Y-%m-%d %H:%M %Z").to_string(),
    })
}
