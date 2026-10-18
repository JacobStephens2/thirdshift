//! Recover Commands that predate records, without inventing stages or liveness.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDateTime, NaiveTime};
use serde::Serialize;

use crate::logs::activity::TIME_FORMAT;
use crate::logs::command_record::Issue;
use crate::progress::ChildLine;

#[derive(Serialize)]
pub(super) struct Command {
    pub repository: String,
    pub(super) kind: String,
    pub(super) issue: Option<Issue>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    outcome: Option<String>,
    pub(super) command_log: Option<PathBuf>,
    pub(super) session_logs: Vec<PathBuf>,
    stages: [(); 0],
    stage: Option<()>,
    source: &'static str,
    pub state: &'static str,
}

impl Command {
    fn from_logs(
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
        }
    }

    pub fn print(&self) {
        let issue = self
            .issue
            .as_ref()
            .map(|issue| format!("#{}", issue.number))
            .unwrap_or_default();
        println!(
            "{}{issue} {} · {} → {} · {}",
            self.repository,
            self.kind,
            self.started_at,
            self.ended_at.as_deref().unwrap_or("unknown end"),
            self.outcome.as_deref().unwrap_or("unknown outcome")
        );
    }
}

struct Started {
    subject: String,
    offset: FixedOffset,
    log: String,
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

pub(super) fn read(repo: &Path, covered: &HashSet<PathBuf>) -> Result<Vec<Command>> {
    let repository = format!(
        "{}/{}",
        repo.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy(),
        repo.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut starts: Vec<Started> = Vec::new();
    for line in read_optional(&repo.join("activity.log"))?.lines() {
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
            if let Some(started) = start(repo, &repository, subject, detail, time)? {
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
    let mut shared_stamps = HashSet::new();
    for start in &starts {
        if let Some(path) = &start.command.command_log {
            covered.insert(path.clone());
            if start.command.kind != "Run"
                && let Some(stamp) = log_stamp(path)
            {
                shared_stamps.insert((path.parent().unwrap().to_path_buf(), stamp));
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
    // still suppress their Session logs, including starts with no end line.
    // `covered` above includes both sources for the Session-log pass.
    commands.extend(sessions(repo, &repository, &covered, &shared_stamps)?);
    Ok(commands)
}

fn start(
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
    // was added. Their known outcome still takes precedence over Session logs.
    let (path, choice) = detail.rsplit_once(", on ").unwrap_or((detail, ""));
    let relative = Path::new(path);
    if !relative.starts_with("commands")
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Ok(None);
    }
    let Some(stamp) = relative
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.get(name.len().checked_sub(20)?..))
        .and_then(parse_stamp)
    else {
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
        log: read_optional(&path)?,
        command,
    }))
}

fn log_stamp(path: &Path) -> Option<String> {
    let name = path.file_stem()?.to_str()?;
    let stamp = name.get(name.len().checked_sub(20)?..)?;
    parse_stamp(stamp)?;
    Some(stamp.into())
}

fn sessions(
    repo: &Path,
    repository: &str,
    covered: &HashSet<PathBuf>,
    shared_stamps: &HashSet<(PathBuf, String)>,
) -> Result<Vec<Command>> {
    let mut groups: BTreeMap<PathBuf, Command> = BTreeMap::new();
    for folder in super::directories(&repo.join("commands"))? {
        for entry in super::entries(&folder)? {
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")) else {
                continue;
            };
            let Some((prefix, rest)) = name.split_once('-') else {
                continue;
            };
            let Some(stamp) = rest.get(..20).and_then(parse_stamp) else {
                continue;
            };
            if rest
                .get(20..)
                .and_then(|rest| rest.strip_prefix('-'))
                .is_none_or(str::is_empty)
            {
                continue;
            }
            let (kind, number) = match prefix.parse::<u64>() {
                Ok(number) => ("Run", Some(number)),
                Err(_) if prefix == "architect" => ("Architect run", None),
                Err(_) if prefix == "secure" => ("Security run", None),
                Err(_) => continue,
            };
            let command_log = folder.join(format!("{prefix}-{}.log", &rest[..20]));
            if covered.contains(&command_log)
                || shared_stamps.contains(&(folder.clone(), rest[..20].into()))
            {
                continue;
            }
            let command = groups.entry(command_log.clone()).or_insert_with(|| {
                let mut command =
                    Command::from_logs(repository, kind, number, stamp, "session_log");
                command.command_log = command_log.is_file().then_some(command_log);
                command
            });
            command.session_logs.push(entry.path());
        }
    }
    for command in groups.values_mut() {
        command.session_logs.sort();
    }
    Ok(groups.into_values().collect())
}

fn parse_stamp(stamp: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%z").ok()
}

fn read_optional(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}
