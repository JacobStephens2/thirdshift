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
use chrono::{DateTime, Local, NaiveDateTime};
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
    pub(super) unavailable_accounts: BTreeSet<String>,
    commands: Vec<Entry>,
    pub(super) idle_passes: Vec<IdlePass>,
    pub(super) pass_starts: Vec<PassStart>,
    now: DateTime<Local>,
    /// The capture its Commands' Session queries and the Logs listing share.
    inventory: Arc<Inventory>,
}

impl Snapshot {
    /// Merge opted-in accounts, isolating another account's read failures.
    pub(super) fn for_user(
        config: &crate::config::UserConfig,
        now: DateTime<Local>,
    ) -> Result<Self> {
        Self::user_capture(config, now, None)
    }

    pub(super) fn details_for_user(
        config: &crate::config::UserConfig,
        now: DateTime<Local>,
        input: &str,
    ) -> Result<Self> {
        Self::user_capture(config, now, Some(input))
    }

    fn user_capture(
        config: &crate::config::UserConfig,
        now: DateTime<Local>,
        input: Option<&str>,
    ) -> Result<Self> {
        let mut snapshot = Self::capture_records(
            Arc::new(inventory::Native),
            &config.logs_dir,
            now,
            config.silent_interval,
            input.is_some(),
        )?;
        let sources = super::accounts::discover(config);
        snapshot.unavailable_accounts = sources.unavailable;
        for source in sources.logs.into_iter().skip(1) {
            let Ok(mut other) = Self::capture_records(
                files_for(&source),
                &source.root,
                now,
                config.silent_interval,
                input.is_some(),
            ) else {
                if matches!(
                    config.console.other_accounts,
                    crate::config::console::OtherAccounts::Named(_)
                ) {
                    snapshot.unavailable_accounts.extend(source.account);
                }
                continue;
            };
            fn mark(command: &mut Command, account: &Option<String>) {
                command.account = account.clone();
                confine_other_account_sessions(command);
                for child in command.children.iter_mut().chain(&mut command.dispatched) {
                    mark(child, account);
                }
            }
            for entry in &mut other.commands {
                match entry {
                    Entry::Record(command) => mark(command, &source.account),
                    Entry::History(command) => command.account = source.account.clone(),
                }
            }
            snapshot.commands.extend(other.commands);
        }
        if let Some(input) = input
            && let Observation::Record(selected) = snapshot.resolve(input)?.observation
        {
            let identity = CommandIdentity::of(selected);
            for entry in &mut snapshot.commands {
                if let Entry::Record(command) = entry {
                    command.add_session_signals(Some(&identity));
                }
            }
        }
        Ok(snapshot)
    }

    pub(super) fn entries(&self) -> &[Entry] {
        &self.commands
    }

    pub(super) fn load(root: &Path, now: DateTime<Local>) -> Result<Self> {
        Self::load_with_silence(root, now, std::time::Duration::from_secs(10 * 60))
    }

    fn load_using(
        files: Arc<dyn inventory::Files>,
        root: &Path,
        now: DateTime<Local>,
    ) -> Result<Self> {
        Self::capture(files, root, now, std::time::Duration::from_secs(10 * 60))
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
            unavailable_accounts: BTreeSet::new(),
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

/// Other accounts stay inside the canonical directory discovery accepted.
fn files_for(source: &super::accounts::Source) -> Arc<dyn inventory::Files> {
    if source.account.is_some() {
        Arc::new(inventory::Confined::new(source.root.clone()))
    } else {
        Arc::new(inventory::Native)
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
pub(crate) struct IdlePass {
    pub(super) repository: String,
    pub(super) kind: String,
    pub(super) reason: String,
    pub(super) recorded_at: Option<NaiveDateTime>,
}

impl IdlePass {
    pub fn timestamp(&self) -> String {
        self.recorded_at.map_or_else(
            || "Time unknown".into(),
            |time| time.format("%Y-%m-%d %H:%M").to_string(),
        )
    }

    pub fn line(&self) -> String {
        format!("{} idle on {}: {}", self.kind, self.repository, self.reason)
    }
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Entry {
    Record(Box<Command>),
    History(Box<history::Command>),
}

#[derive(Serialize)]
pub(super) struct Command {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) account: Option<String>,
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
    /// Command's exact identity. A match also enriches nested Ticket
    /// runs and dispatches. Path alone is not enough: a dispatch shares its
    /// parent's record path. Usual ranges and health already include every record.
    fn add_session_signals(&mut self, selected: Option<&CommandIdentity>) {
        let enrich = selected.is_none_or(|identity| identity.matches(self));
        if enrich && let Some(mut metrics) = self.record.run_metrics.take() {
            metrics.add_session_signals(
                &self.record,
                if self.account.is_some() {
                    cache::readonly_session_signals
                } else {
                    cache::session_signals
                },
            );
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
            account: None,
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

/// Another account's record names its Session path. Keep a trace only when
/// every file it is read from is a regular file whose canonical path stays
/// inside that record's own directory. A lexical prefix is not enough: `..`
/// and a symlink can leave.
fn confine_other_account_sessions(command: &mut Command) {
    let root = command.record_path.parent();
    let keep = |path: &Path| {
        let Some(root) = root else {
            return false;
        };
        contained_session(root, path)
    };
    if command
        .record
        .session_log
        .as_deref()
        .is_some_and(|path| !keep(path))
    {
        command.record.session_log = None;
    }
    for stage in &mut command.record.stages {
        if stage.session_log.as_deref().is_some_and(|path| !keep(path)) {
            stage.session_log = None;
        }
    }
    command.activity.drop_session_unless(keep);
}

/// The logs and spans files opened for one Session trace. A missing sidecar is
/// not an escape; a symlink or a resolved path outside `root` is.
fn contained_session(root: &Path, path: &Path) -> bool {
    if !contained_trace(root, path) {
        return false;
    }
    let original = super::session::original(path);
    [
        original.with_extension("logs.jsonl"),
        original.with_extension("spans.jsonl"),
    ]
    .into_iter()
    .all(|sidecar| sidecar == path || absent_or_contained(root, &sidecar))
}

fn absent_or_contained(root: &Path, path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => contained_trace(root, path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn contained_trace(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return false;
    }
    // `symlink_metadata` follows every component except the last, so a
    // directory symlink still looks like a regular file. Resolve it.
    if !std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return false;
    }
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    path.canonicalize()
        .is_ok_and(|canonical| canonical.starts_with(&root))
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
        if let Some((time, line)) = activity::last_of(&activity, kind)
            && let Some(reason) = line.strip_prefix(&format!("{kind} skipped: "))
        {
            idle.push(IdlePass {
                repository: repository.clone(),
                kind: kind.to_string(),
                reason: reason.into(),
                recorded_at: NaiveDateTime::parse_from_str(time, activity::TIME_FORMAT).ok(),
            });
        }
    }
    Ok(idle)
}
