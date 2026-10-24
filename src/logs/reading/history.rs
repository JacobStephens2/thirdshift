//! Recover Commands that predate records, without inventing stages or liveness.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDateTime, NaiveTime};
use serde::Serialize;

use super::inventory::{Inventory, Repository};
use crate::logs::activity::TIME_FORMAT;
use crate::logs::command_record::Issue;
use crate::logs::owner;
use crate::progress::ChildLine;

#[derive(Clone, Serialize)]
pub(in crate::logs) struct Command {
    pub repository: String,
    pub(in crate::logs) kind: String,
    pub(in crate::logs) issue: Option<Issue>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub(in crate::logs) outcome: Option<String>,
    pub(in crate::logs) command_log: Option<PathBuf>,
    pub(in crate::logs) session_logs: Vec<PathBuf>,
    stages: [(); 0],
    stage: Option<()>,
    source: &'static str,
    pub state: &'static str,
    #[serde(skip)]
    pub(super) inventory: Arc<Inventory>,
}

impl Command {
    fn from_logs(
        inventory: &Arc<Inventory>,
        repository: &str,
        kind: &str,
        number: Option<u64>,
        started: DateTime<FixedOffset>,
        source: &'static str,
    ) -> Self {
        Self {
            repository: repository.into(),
            kind: kind.into(),
            issue: number.map(|number| Issue {
                number,
                url: format!("https://github.com/{repository}/issues/{number}"),
                title: None,
            }),
            harness: None,
            model: None,
            effort: None,
            started_at: started.to_rfc3339(),
            ended_at: None,
            outcome: None,
            command_log: None,
            session_logs: Vec::new(),
            stages: [],
            stage: None,
            source,
            state: "unknown",
            inventory: inventory.clone(),
        }
    }
}

struct Started {
    subject: String,
    offset: FixedOffset,
    log: std::sync::Arc<str>,
    command: Command,
}

impl Started {
    fn ending_distance(&self, time: NaiveDateTime, summary: &str) -> Option<i64> {
        let ending = summary
            .strip_prefix("plan ")
            .and_then(|summary| summary.split_once(" dispatched: "))
            .map_or(summary, |(_, ending)| ending);
        let ending = ending
            .strip_prefix("failed: ")
            .or_else(|| ending.strip_prefix("review required: "))
            .unwrap_or(ending);
        self.log
            .lines()
            .filter_map(|line| {
                let ChildLine::Own(message) = ChildLine::of(line) else {
                    return None;
                };
                // Activity summaries wrap a failure's first line and append recovery
                // facts; terminal output prints the cause and those facts separately.
                if message.is_empty()
                    || !(summary == message
                        || ending == message
                        || ending
                            .strip_prefix(message)
                            .is_some_and(|suffix| suffix.starts_with("; ")))
                {
                    return None;
                }
                let clock = line
                    .strip_prefix("thirdshift: ")
                    .and_then(|line| line.split_once(' '))
                    .and_then(|(clock, _)| NaiveTime::parse_from_str(clock, "%H:%M:%S").ok());
                // The terminal ending follows the Activity write, possibly across a
                // second or midnight. Its nearest clock disambiguates equal causes.
                Some(clock.map_or(i64::MAX, |clock| {
                    clock
                        .signed_duration_since(time.time())
                        .num_seconds()
                        .rem_euclid(24 * 3600)
                }))
            })
            .min()
    }
}

pub(in crate::logs) fn read(
    inventory: &Arc<Inventory>,
    captured: &Repository,
    covered: &HashSet<PathBuf>,
) -> Result<Vec<Command>> {
    let repo = captured.path.as_path();
    let repository = format!(
        "{}/{}",
        repo.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy(),
        repo.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut starts: Vec<Started> = Vec::new();
    for line in super::cache::text(&repo.join("activity.log"))?.lines() {
        let Some((time, text)) = line.split_once(' ').and_then(|(date, rest)| {
            let (clock, text) = rest.split_once(' ')?;
            Some((
                NaiveDateTime::parse_from_str(&format!("{date} {clock}"), TIME_FORMAT).ok()?,
                text,
            ))
        }) else {
            continue;
        };
        if let Some((subject, detail)) = text.split_once(" started: ") {
            if let Some(started) = start(inventory, repo, &repository, subject, detail, time)? {
                starts.push(started);
            }
        } else if let Some((subject, outcome)) = text.split_once(" ended: ") {
            // A subject may have concurrent Commands. Prefer the Command log
            // containing this ending over an assumed start/end pairing order.
            let candidates: Vec<_> = starts
                .iter()
                .enumerate()
                .filter(|(_, start)| start.subject == subject && start.command.ended_at.is_none())
                .map(|(index, _)| index)
                .collect();
            let matches: Vec<_> = candidates
                .iter()
                .filter_map(|&index| {
                    starts[index]
                        .ending_distance(time, outcome)
                        .map(|distance| (index, distance))
                })
                .collect();
            let matched = matches
                .iter()
                .min_by_key(|(_, distance)| distance)
                .filter(|(_, distance)| {
                    matches
                        .iter()
                        .filter(|(_, other)| other == distance)
                        .count()
                        == 1
                })
                .map(|&(index, _)| index);
            let index = matched.or_else(|| (candidates.len() == 1).then(|| candidates[0]));
            if let Some(index) = index {
                let start = &mut starts[index];
                start.command.ended_at = time
                    .and_local_timezone(start.offset)
                    .single()
                    .map(|time| time.to_rfc3339());
                start.command.outcome = Some(outcome.into());
                start.command.state = "ended";
            }
        }
    }
    let recorded = covered;
    let mut covered = recorded.clone();
    let mut shared_owners = Vec::new();
    for start in &starts {
        if let Some(path) = &start.command.command_log {
            covered.insert(path.clone());
            if start.command.kind != "Run" && owner::decode(path).is_some() {
                shared_owners.push(path.clone());
            }
        }
    }
    let mut commands: Vec<_> = starts
        .into_iter()
        .map(|start| start.command)
        .filter(|command| {
            command
                .command_log
                .as_ref()
                .is_none_or(|path| !recorded.contains(path))
        })
        .collect();
    // Keep Activity entries unless a record covered them; all Activity starts
    // still suppress their Session traces, including starts with no end line.
    // `covered` above includes both sources for the Session-trace pass.
    commands.extend(sessions(
        inventory,
        captured,
        &repository,
        &covered,
        &shared_owners,
    ));
    Ok(commands)
}

fn start(
    inventory: &Arc<Inventory>,
    repo: &Path,
    repository: &str,
    subject: &str,
    detail: &str,
    time: NaiveDateTime,
) -> Result<Option<Started>> {
    let (kind, number) = match subject.rsplit_once(" #") {
        Some((kind, number)) => (kind, number.parse::<u64>().ok()),
        None => (subject, None),
    };
    if !matches!(
        kind,
        "Run" | "Spec run" | "Pickup run" | "Architect run" | "Security run"
    ) {
        return Ok(None);
    }
    // Early Activity logs named only the Command log, before Harness metadata
    // was added. Their known outcome still takes precedence over Session traces.
    let (path, choice) = detail.rsplit_once(", on ").unwrap_or((detail, ""));
    let relative = Path::new(path);
    if !relative.starts_with("commands")
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Ok(None);
    }
    let Some((_, stamp)) = super::command_stamp(relative) else {
        return Ok(None);
    };
    let path = repo.join(relative);
    let mut choice = choice.split(" · ");
    let harness = choice
        .next()
        .filter(|name| !name.is_empty())
        .map(String::from);
    let model = choice
        .next()
        .filter(|value| *value != "default model")
        .map(String::from);
    let effort = choice
        .next()
        .filter(|value| *value != "default effort")
        .map(String::from);
    let mut command = Command::from_logs(
        inventory,
        repository,
        kind,
        number,
        time.and_local_timezone(*stamp.offset()).single().unwrap(),
        "activity_log",
    );
    command.harness = harness;
    command.model = model;
    command.effort = effort;
    command.command_log = Some(path.clone());
    Ok(Some(Started {
        subject: subject.into(),
        offset: *stamp.offset(),
        log: super::cache::text(&path)?,
        command,
    }))
}

fn sessions(
    inventory: &Arc<Inventory>,
    repo: &Repository,
    repository: &str,
    covered: &HashSet<PathBuf>,
    shared_owners: &[PathBuf],
) -> Vec<Command> {
    let mut groups: BTreeMap<PathBuf, Command> = BTreeMap::new();
    for folder in &repo.folders {
        for session in inventory.candidates(folder) {
            let (kind, number) = match session.subject.parse::<u64>() {
                Ok(number) => ("Run", Some(number)),
                Err(_) if session.subject == "architect" => ("Architect run", None),
                Err(_) if session.subject == "secure" => ("Security run", None),
                Err(_) => continue,
            };
            let command_log = session.command_log();
            if covered.contains(&session.path)
                || covered.contains(command_log)
                || shared_owners
                    .iter()
                    .any(|log| log.parent() == Some(folder.path.as_path()) && session.owned_by(log))
            {
                continue;
            }
            let command = groups.entry(command_log.to_owned()).or_insert_with(|| {
                let mut command = Command::from_logs(
                    inventory,
                    repository,
                    kind,
                    number,
                    session.time,
                    "session_log",
                );
                command.command_log = inventory
                    .is_file(command_log)
                    .then(|| command_log.to_owned());
                command
            });
            command.session_logs.push(session.path.clone());
        }
    }
    for command in groups.values_mut() {
        command.session_logs.sort();
    }
    groups.into_values().collect()
}
