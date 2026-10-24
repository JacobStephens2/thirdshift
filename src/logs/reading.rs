//! Filesystem observations of Commands, their Runs and exact ownership.
//! Loading uses one sampled time; reading never prints or performs Stop effects.

mod cache;
pub(crate) mod catalog;
mod health;
mod history;
mod inbox;
mod reader;
mod selection;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::Serialize;

use super::command_record::{CommandRecord, CommandTree};
use super::effects::OnMachine;
use super::{Pass, activity};
pub(super) use health::{Health, UsualRange};
pub(super) use history::Command as HistoricalCommand;
pub(super) use inbox::{Group, InboxItem};
pub(crate) use reader::{Framing, Reader, Writer};
pub(super) use selection::{Detailed, Observation};

pub(super) struct Snapshot {
    commands: Vec<Entry>,
    pub(super) idle_passes: Vec<IdlePass>,
    now: DateTime<Local>,
}

impl Snapshot {
    pub(super) fn next_inbox_change(&self, days: i64) -> Option<DateTime<Local>> {
        inbox::next_change(&self.commands, self.now, days)
    }

    pub(super) fn entries(&self) -> &[Entry] {
        &self.commands
    }

    pub(super) fn load(root: &Path, now: DateTime<Local>) -> Result<Self> {
        Self::load_with_silence(root, now, std::time::Duration::from_secs(10 * 60))
    }

    pub(super) fn load_with_silence(
        root: &Path,
        now: DateTime<Local>,
        silent: std::time::Duration,
    ) -> Result<Self> {
        let mut records = BTreeMap::new();
        let mut commands = Vec::new();
        let mut idle_passes = Vec::new();
        for owner in directories(root)? {
            for repo in directories(&owner)? {
                let mut covered = HashSet::new();
                idle_passes.extend(read_idle_passes(&repo)?);
                for folder in directories(&repo.join("commands"))? {
                    for (path, tree) in read_records(&folder)? {
                        let record = &tree.command;
                        if record.parent_record.is_none() {
                            covered.insert(command_log(&path));
                        }
                        for session in std::iter::once(record)
                            .chain(&tree.dispatched)
                            .flat_map(CommandRecord::session_references)
                        {
                            covered.insert(session.clone());
                            covered.insert(super::session::path(session));
                        }
                        records.insert(path, tree);
                    }
                }
                commands.extend(
                    history::read(&repo, &covered)?
                        .into_iter()
                        .map(|command| Entry::History(Box::new(command))),
                );
            }
        }
        let ranges = health::UsualRanges::from_records(&records);
        let roots: Vec<_> = records
            .iter()
            .filter(|(_, tree)| {
                let record = &tree.command;
                record.parent_record.is_none()
            })
            .map(|(path, _)| path.clone())
            .collect();
        commands.extend(
            roots
                .into_iter()
                .map(|path| Entry::Record(Box::new(tree(&path, &mut records, &now)))),
        );
        for entry in &mut commands {
            if let Entry::Record(command) = entry {
                health::observe(command, &now, silent, None, &ranges);
            }
        }
        idle_passes.sort_by(|a, b| (&a.repository, &a.kind).cmp(&(&b.repository, &b.kind)));
        Ok(Self {
            commands,
            idle_passes,
            now,
        })
    }

    pub(super) fn inbox(&self, all: bool, days: i64) -> Vec<InboxItem<'_>> {
        inbox::list(&self.commands, self.now, all, days)
    }

    pub(super) fn resolve(&self, input: &str) -> Result<selection::Selection<'_>> {
        selection::resolve(&self.commands, input)
    }
}

/// Reload only the captured Command's folder, after Stop cleanup has finished.
pub(super) fn retained_work(path: &Path, now: DateTime<Local>) -> Result<RetainedWork> {
    let mut records = read_records(path.parent().context("Command record has no directory")?)?;
    anyhow::ensure!(records.contains_key(path), "Command record is unavailable");
    let command = tree(path, &mut records, &now);
    let mut work = RetainedWork::default();
    fn collect(command: &Command, work: &mut RetainedWork) {
        work.branches
            .extend(command.record.issue_branch.iter().cloned());
        work.pull_requests
            .extend(command.record.pull_request.iter().cloned());
        for child in command.dispatched.iter().chain(&command.children) {
            collect(child, work);
        }
    }
    collect(&command, &mut work);
    Ok(work)
}

#[derive(Default)]
pub(super) struct RetainedWork {
    pub(super) branches: BTreeSet<String>,
    pub(super) pull_requests: BTreeSet<String>,
}

#[derive(Serialize)]
pub(super) struct IdlePass {
    pub(super) repository: String,
    pub(super) kind: String,
    pub(super) reason: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Entry {
    Record(Box<Command>),
    History(Box<history::Command>),
}

#[derive(Serialize)]
pub(super) struct Command {
    #[serde(skip)]
    pub(super) record_path: PathBuf,
    #[serde(flatten)]
    pub(super) record: CommandRecord,
    pub(super) state: &'static str,
    pub(super) stage: Option<CurrentStage>,
    pub(super) children: Vec<Command>,
    pub(super) source: &'static str,
    pub(super) command_log: PathBuf,
    pub(super) dispatched: Vec<Command>,
    pub(super) health: Option<Health>,
    #[serde(skip)]
    pub(super) next_health_change: Option<DateTime<Local>>,
    pub(super) usual_ranges: BTreeMap<String, UsualRange>,
}

#[derive(Serialize)]
pub(super) struct CurrentStage {
    pub(super) name: String,
    pub(super) began_at: String,
    pub(super) age_seconds: i64,
}

impl Command {
    pub(in crate::logs) fn new(
        record: CommandRecord,
        record_path: PathBuf,
        now: DateTime<Local>,
    ) -> Self {
        let state = if record.outcome.is_some() {
            "ended"
        } else if record.live(&mut OnMachine) {
            "live"
        } else {
            "died"
        };
        let stage = record.current_stage().map(|stage| CurrentStage {
            name: stage.name.clone(),
            began_at: stage.began_at.clone(),
            age_seconds: DateTime::parse_from_rfc3339(&stage.began_at)
                .map(|began| now.signed_duration_since(began).num_seconds().max(0))
                .unwrap_or(0),
        });
        Self {
            command_log: command_log(&record_path),
            record_path,
            record,
            state,
            stage,
            children: Vec::new(),
            source: "command_record",
            dispatched: Vec::new(),
            health: None,
            next_health_change: None,
            usual_ranges: BTreeMap::new(),
        }
    }
}

fn read_records(folder: &Path) -> Result<BTreeMap<PathBuf, CommandTree>> {
    let mut records = BTreeMap::new();
    for entry in entries(folder)? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_file()
            || (!name.ends_with(".command.json") && !name.ends_with(".child.json"))
        {
            continue;
        }
        let path = entry.path();
        let Some(mut tree) = cache::record(&path)? else {
            continue;
        };
        let record = &mut tree.command;
        if record.version != 1 {
            continue;
        }
        // Records written before completion times existed were last
        // replaced when their outcome was recorded.
        if record.outcome.is_some() && record.ended_at.is_none() {
            record.end_time_estimated = true;
            record.ended_at = entry
                .metadata()?
                .modified()
                .ok()
                .map(|time| DateTime::<Local>::from(time).to_rfc3339());
        }
        records.insert(path, tree);
    }
    Ok(records)
}

fn command_log(path: &Path) -> PathBuf {
    super::owner::record_log(path)
}

/// Keep the filename spelling alongside its parsed time for naming and history.
fn command_stamp(path: &Path) -> Option<(&str, DateTime<chrono::FixedOffset>)> {
    let decoded = super::owner::decode(path)?;
    Some((decoded.identity.stamp, decoded.time()))
}

fn tree(
    path: &Path,
    records: &mut BTreeMap<PathBuf, CommandTree>,
    now: &DateTime<Local>,
) -> Command {
    let tree = records.remove(path).expect("a discovered record exists");
    let mut command = Command::new(tree.command, path.to_owned(), *now);
    command.children = children(path, None, records, now);
    command.dispatched = tree
        .dispatched
        .into_iter()
        .map(|record| {
            let mut dispatched = Command::new(record, path.to_owned(), *now);
            let issue = dispatched
                .record
                .issue
                .as_ref()
                .map(|issue| issue.url.as_str());
            dispatched.children = children(path, issue, records, now);
            dispatched
        })
        .collect();
    command
}

fn children(
    path: &Path,
    dispatch: Option<&str>,
    records: &mut BTreeMap<PathBuf, CommandTree>,
    now: &DateTime<Local>,
) -> Vec<Command> {
    let paths: Vec<_> = records
        .iter()
        .filter(|(_, child)| {
            child.command.parent_record.as_deref() == Some(path)
                && child.command.parent_dispatch.as_deref() == dispatch
        })
        .map(|(path, _)| path.clone())
        .collect();
    let mut children: Vec<_> = paths
        .into_iter()
        .map(|path| tree(&path, records, now))
        .collect();
    children.sort_by_key(|child| child.record.issue.as_ref().map(|issue| issue.number));
    children
}

fn read_idle_passes(repo: &Path) -> Result<Vec<IdlePass>> {
    let path = repo.join(activity::FILE);
    let activity = cache::text(&path)?;
    let repository = format!(
        "{}/{}",
        repo.parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
        repo.file_name().unwrap().to_string_lossy()
    );
    let mut idle = Vec::new();
    for pass in [Pass::Architect, Pass::Pickup, Pass::Security] {
        let kind = activity::Kind::Pass(pass);
        if let Some(line) = activity::last_of(&activity, kind)
            && let Some(reason) = line.strip_prefix(&format!("{kind} skipped: "))
        {
            idle.push(IdlePass {
                repository: repository.clone(),
                kind: kind.to_string(),
                reason: reason.into(),
            });
        }
    }
    Ok(idle)
}

fn entries(path: &Path) -> Result<Vec<fs::DirEntry>> {
    match fs::read_dir(path) {
        Ok(entries) => entries.collect::<std::io::Result<_>>().map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("could not list {}", path.display())),
    }
}

fn directories(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in entries(path)? {
        if entry.file_type()?.is_dir() {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}
