//! Recover Commands that predate records, without inventing stages or liveness.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde::Serialize;

use crate::logs::activity::TIME_FORMAT;
use crate::logs::command_record::Issue;
use crate::progress::ChildLine;

#[derive(Serialize)]
pub(super) struct Command {
    pub repository: String,
    kind: String,
    issue: Option<Issue>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    outcome: Option<String>,
    command_log: Option<PathBuf>,
    session_logs: Vec<PathBuf>,
    stages: [(); 0],
    stage: Option<()>,
    source: &'static str,
    pub state: &'static str,
}

impl Command {
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
            let matched = candidates.iter().copied().find(|&index| starts[index].log.lines().any(|line| matches!(ChildLine::of(line), ChildLine::Own(message) if message == outcome)));
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
    let Some((path, choice)) = detail.rsplit_once(", on ") else {
        return Ok(None);
    };
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
    let harness = choice.next().map(String::from);
    let model = choice
        .next()
        .filter(|value| *value != "default model")
        .map(String::from);
    let effort = choice
        .next()
        .filter(|value| *value != "default effort")
        .map(String::from);
    Ok(Some(Started {
        subject: subject.into(),
        offset: *stamp.offset(),
        log: read_optional(&path)?,
        command: Command {
            repository: repository.into(),
            kind: kind.into(),
            issue: number.map(|number| Issue {
                number,
                url: format!("https://github.com/{repository}/issues/{number}"),
                title: None,
            }),
            harness,
            model,
            effort,
            started_at: time
                .and_local_timezone(*stamp.offset())
                .single()
                .unwrap()
                .to_rfc3339(),
            ended_at: None,
            outcome: None,
            command_log: Some(path),
            session_logs: Vec::new(),
            stages: [],
            stage: None,
            source: "activity_log",
            state: "unknown",
        },
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
            let command = groups
                .entry(command_log.clone())
                .or_insert_with(|| Command {
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
                    started_at: stamp.to_rfc3339(),
                    ended_at: None,
                    outcome: None,
                    command_log: command_log.is_file().then_some(command_log),
                    session_logs: Vec::new(),
                    stages: [],
                    stage: None,
                    source: "session_log",
                    state: "unknown",
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
