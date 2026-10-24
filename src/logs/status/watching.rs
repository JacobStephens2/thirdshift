//! Dependencies that can change a live observation without a record write.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Local};

use super::super::reading::{Command, Entry, Snapshot};
use crate::config::UserConfig;

pub(crate) use super::super::reading::Writer as Process;

#[derive(Default, Debug)]
pub(crate) struct Watching {
    pub processes: Vec<Process>,
    pub paths: BTreeSet<PathBuf>,
    pub deadline: Option<Duration>,
}

pub(crate) fn watching(config: &UserConfig) -> Result<Watching> {
    let now = Local::now();
    let snapshot = Snapshot::load_with_silence(&config.logs_dir, now, config.silent_interval)?;
    let mut watching = Watching::default();
    let kept = crate::logs::github::load(&config.logs_dir);
    watching.deadline = kept.next_warning_change();
    if let Some(at) = snapshot
        .evaluate_inbox(config.inbox_days, &kept)
        .next_change()
    {
        deadline(at, now, &mut watching);
    }
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry {
            collect(command, now, &mut watching);
        }
    }
    Ok(watching)
}

fn collect(command: &Command, now: DateTime<Local>, watching: &mut Watching) {
    let process = Process::capture(&command.record);
    if process.live() {
        watching.processes.push(process);
        watching
            .paths
            .extend(command.activity_paths().map(PathBuf::from));
    }
    if command.state == "live"
        && let Some(at) = command.next_health_change
    {
        deadline(at, now, watching);
    }
    for child in command.children.iter().chain(&command.dispatched) {
        collect(child, now, watching);
    }
}

fn deadline(at: DateTime<Local>, now: DateTime<Local>, watching: &mut Watching) {
    if let Ok(delay) = (at - now).to_std()
        && !delay.is_zero()
    {
        watching.deadline = Some(watching.deadline.map_or(delay, |old| old.min(delay)));
    }
}
