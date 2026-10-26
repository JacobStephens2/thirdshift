//! Filesystem observations of Commands, their Runs and exact ownership.
//! Loading uses one sampled time; reading never prints or performs Stop effects.

mod attribution;
mod cache;
mod captured_activity;
pub(crate) mod catalog;
mod health;
mod history;
mod inbox;
mod inventory;
mod reader;
mod selection;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::Serialize;

use super::command_record::{CommandRecord, CommandTree};
use super::effects::OnMachine;
use super::pass_starts::{self, PassStart};
pub(super) use super::usual_range::UsualRange;
use super::{Pass, activity};
pub(super) use attribution::{CommandIdentity, PullRequestAttribution};
pub(super) use health::Health;
pub(super) use history::Command as HistoricalCommand;
pub(super) use inbox::{Group, InboxItem};
use inventory::{Folder, Inventory};
pub(crate) use reader::{Framing, Reader, Writer};
pub(super) use selection::Observation;

pub(super) struct Snapshot {
    commands: Vec<Entry>,
    pub(super) idle_passes: Vec<IdlePass>,
    pub(super) pass_starts: Vec<PassStart>,
    now: DateTime<Local>,
    /// The capture its Commands' Session queries and the Logs listing share.
    inventory: Arc<Inventory>,
}

impl Snapshot {
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
        Self::capture(Arc::new(inventory::Native), root, now, silent)
    }

    pub(super) fn load_metrics(root: &Path, now: DateTime<Local>) -> Result<Self> {
        let mut snapshot = Self::capture_records(
            Arc::new(inventory::Native),
            root,
            now,
            std::time::Duration::from_secs(600),
            true,
        )?;
        for entry in &mut snapshot.commands {
            if let Entry::Record(command) = entry {
                command.add_session_signals(None);
            }
        }
        Ok(snapshot)
    }

    pub(super) fn load_details(
        root: &Path,
        now: DateTime<Local>,
        silent: std::time::Duration,
        input: &str,
    ) -> Result<Self> {
        let mut snapshot =
            Self::capture_records(Arc::new(inventory::Native), root, now, silent, true)?;
        if let Observation::Record(selected) = snapshot.resolve(input)?.observation {
            let path = selected.record_path.clone();
            let name = selected.record.name.clone();
            for entry in &mut snapshot.commands {
                if let Entry::Record(command) = entry {
                    command.add_session_signals(Some((&path, &name)));
                }
            }
        }
        Ok(snapshot)
    }

    fn capture(
        files: Arc<dyn inventory::Files>,
        root: &Path,
        now: DateTime<Local>,
        silent: std::time::Duration,
    ) -> Result<Self> {
        Self::capture_records(files, root, now, silent, false)
    }

    fn capture_records(
        files: Arc<dyn inventory::Files>,
        root: &Path,
        now: DateTime<Local>,
        silent: std::time::Duration,
        past_metrics: bool,
    ) -> Result<Self> {
        let inventory = Arc::new(Inventory::capture(files, root)?);
        let mut records = BTreeMap::new();
        let mut commands = Vec::new();
        let mut idle_passes = Vec::new();
        let pass_starts = starts_in(&inventory, now)?;
        for repo in inventory.repositories() {
            let mut covered = HashSet::new();
            idle_passes.extend(read_idle_passes(&repo.path)?);
            for folder in &repo.folders {
                for (path, tree) in read_records(&inventory, folder, past_metrics)? {
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
                history::read(&inventory, repo, &covered)?
                    .into_iter()
                    .map(|command| Entry::History(Box::new(command))),
            );
        }
        let ranges = super::usual_range::UsualRanges::from_records(&records);
        let roots: Vec<_> = records
            .iter()
            .filter(|(_, tree)| {
                let record = &tree.command;
                record.parent_record.is_none()
            })
            .map(|(path, _)| path.clone())
            .collect();
        commands.extend(roots.into_iter().map(|path| {
            Entry::Record(Box::new(tree(&inventory, &path, None, &mut records, &now)))
        }));
        for entry in &mut commands {
            if let Entry::Record(command) = entry {
                health::observe(command, &now, silent, &ranges);
            }
        }
        idle_passes.sort_by(|a, b| (&a.repository, &a.kind).cmp(&(&b.repository, &b.kind)));
        Ok(Self {
            commands,
            idle_passes,
            pass_starts,
            now,
            inventory,
        })
    }

    pub(super) fn evaluate_inbox(
        &self,
        days: i64,
        github: &super::github::Kept,
    ) -> inbox::Evaluation<'_> {
        inbox::evaluate(&self.commands, self.now, days, github)
    }

    pub(super) fn active_runs(&self) -> usize {
        inbox::active_runs(&self.commands)
    }

    pub(super) fn resolve(&self, input: &str) -> Result<selection::Selection<'_>> {
        selection::resolve(&self.commands, input)
    }
}

/// Start times alone, without reading unrelated Command records or GitHub.
pub(super) fn pass_starts(root: &Path, now: DateTime<Local>) -> Result<Vec<PassStart>> {
    let inventory = Inventory::capture(Arc::new(inventory::Native), root)?;
    starts_in(&inventory, now)
}

fn starts_in(inventory: &Inventory, now: DateTime<Local>) -> Result<Vec<PassStart>> {
    let mut starts = Vec::new();
    for repo in inventory.repositories() {
        starts.extend(pass_starts::read(&repo.path, &repo.name(), now)?);
    }
    starts.sort_by(|a, b| (&a.repository, &a.kind).cmp(&(&b.repository, &b.kind)));
    Ok(starts)
}

/// Match a PR to the exact retained Run, including child and dispatched Runs.
pub(crate) fn run_for_pull_request(root: &Path, url: &str) -> Result<Option<(PathBuf, String)>> {
    let snapshot = Snapshot::load(root, Local::now())?;
    Ok(PullRequestAttribution::new(&snapshot)
        .latest(url)
        .map(|command| (command.record_path.clone(), command.name.clone())))
}

/// Reload only the captured Command's folder, after Stop cleanup has finished.
pub(super) fn retained_work(path: &Path, now: DateTime<Local>) -> Result<RetainedWork> {
    let inventory = Arc::new(Inventory::on_demand());
    let folder = inventory.folder(path.parent().context("Command record has no directory")?)?;
    let mut records = read_records(&inventory, &folder, false)?;
    anyhow::ensure!(records.contains_key(path), "Command record is unavailable");
    let command = tree(&inventory, path, None, &mut records, &now);
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
    #[serde(skip)]
    inventory: Arc<Inventory>,
    #[serde(skip)]
    activity: captured_activity::Activity,
}

#[derive(Serialize)]
pub(super) struct CurrentStage {
    pub(super) name: String,
    pub(super) began_at: String,
    pub(super) age_seconds: i64,
}

impl Command {
    /// Enrich this Command when `selected` is every Command (`None`), or this
    /// Command's own record path and name. A match also enriches nested Ticket
    /// runs and dispatches. Path alone is not enough: a dispatch shares its
    /// parent's record path. Usual ranges and health already include every record.
    fn add_session_signals(&mut self, selected: Option<(&Path, &str)>) {
        let enrich = selected
            .is_none_or(|(path, name)| self.record_path == path && self.record.name == name);
        if enrich && let Some(mut metrics) = self.record.run_metrics.take() {
            metrics.add_session_signals(&self.record, cache::session_signals);
            self.record.run_metrics = Some(metrics);
        }
        let nested = if enrich { None } else { selected };
        for child in self.dispatched.iter_mut().chain(&mut self.children) {
            child.add_session_signals(nested);
        }
    }

    fn new(
        inventory: &Arc<Inventory>,
        record: CommandRecord,
        record_path: PathBuf,
        now: DateTime<Local>,
        log: &Path,
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
            activity: captured_activity::Activity::new(log, record.session_log.as_deref(), now),
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
            inventory: inventory.clone(),
        }
    }
}

pub(super) fn repository_records(root: &Path) -> Result<BTreeMap<PathBuf, CommandTree>> {
    let inventory = Inventory::on_demand();
    let mut records = BTreeMap::new();
    for folder in inventory.directories(&root.join("commands"))? {
        let folder = inventory.folder(&folder)?;
        records.extend(read_records(&inventory, &folder, false)?);
    }
    Ok(records)
}

fn read_records(
    inventory: &Inventory,
    folder: &Folder,
    past_metrics: bool,
) -> Result<BTreeMap<PathBuf, CommandTree>> {
    let mut records = BTreeMap::new();
    for path in &folder.files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !name.ends_with(".command.json") && !name.ends_with(".child.json") {
            continue;
        }
        let Some(mut tree) = cache::record(path)? else {
            continue;
        };
        if !matches!(tree.command.version, 1 | 2) {
            continue;
        }
        if past_metrics {
            for record in std::iter::once(&mut tree.command).chain(&mut tree.dispatched) {
                if record.run_metrics.is_none() && record.outcome.is_some() {
                    record.run_metrics = Some(super::run_metrics::RunMetrics::historical(record));
                }
            }
        }
        let record = &mut tree.command;
        // Records written before completion times existed were last
        // replaced when their outcome was recorded. Keep that health fallback
        // after metric reconstruction so it cannot invent recorded durations.
        if record.outcome.is_some() && record.ended_at.is_none() {
            record.end_time_estimated = true;
            record.ended_at = inventory
                .modified(path)?
                .map(|time| DateTime::<Local>::from(time).to_rfc3339());
        }
        records.insert(path.clone(), tree);
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
    inventory: &Arc<Inventory>,
    path: &Path,
    outer_log: Option<&Path>,
    records: &mut BTreeMap<PathBuf, CommandTree>,
    now: &DateTime<Local>,
) -> Command {
    let tree = records.remove(path).expect("a discovered record exists");
    let log = outer_log.map_or_else(|| command_log(path), Path::to_path_buf);
    let mut command = Command::new(inventory, tree.command, path.to_owned(), *now, &log);
    command.children = children(inventory, path, None, &log, records, now);
    command.dispatched = tree
        .dispatched
        .into_iter()
        .map(|record| {
            let mut dispatched = Command::new(inventory, record, path.to_owned(), *now, &log);
            let issue = dispatched
                .record
                .issue
                .as_ref()
                .map(|issue| issue.url.as_str());
            dispatched.children = children(inventory, path, issue, &log, records, now);
            dispatched
        })
        .collect();
    command
}

fn children(
    inventory: &Arc<Inventory>,
    path: &Path,
    dispatch: Option<&str>,
    log: &Path,
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
        .map(|path| tree(inventory, &path, Some(log), records, now))
        .collect();
    children.sort_by_key(|child| child.record.issue.as_ref().map(|issue| issue.number));
    children
}

fn read_idle_passes(repo: &Path) -> Result<Vec<IdlePass>> {
    let path = repo.join(activity::FILE);
    let activity = cache::text(&path)?;
    let repository = inventory::name(repo);
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
