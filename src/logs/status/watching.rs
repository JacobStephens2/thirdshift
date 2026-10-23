//! Dependencies that can change a live observation without a record write.
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Local};

use super::super::reading::{Command, Entry, Snapshot};
use crate::config::UserConfig;

#[derive(Clone, Debug)]
pub(crate) struct Process {
    pub pid: u32,
    start: String,
}

impl Process {
    pub fn live(&self) -> bool {
        super::super::process_identity::start(self.pid).as_deref() == Some(&self.start)
    }
}

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
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry {
            collect(command, None, config, now, &mut watching);
        }
    }
    Ok(watching)
}

fn collect(
    command: &Command,
    outer_log: Option<&std::path::Path>,
    config: &UserConfig,
    now: DateTime<Local>,
    watching: &mut Watching,
) {
    let log = outer_log.unwrap_or(&command.command_log);
    if command.state == "live" {
        watching.processes.push(Process {
            pid: command.record.process_id,
            start: command.record.process_start.clone(),
        });
        let session = command
            .record
            .session_log
            .as_deref()
            .map(crate::logs::session::path);
        watching.paths.insert(log.to_path_buf());
        watching.paths.extend(session.iter().cloned());
        if let Some(stage) = &command.stage
            && let Ok(began) = DateTime::parse_from_rfc3339(&stage.began_at)
        {
            let mut last_output = began.with_timezone(&Local);
            for path in std::iter::once(log).chain(session.as_deref()) {
                if let Ok(modified) = std::fs::metadata(path).and_then(|file| file.modified()) {
                    last_output = last_output.max(DateTime::from(modified));
                }
            }
            for multiple in [1, 3] {
                deadline(
                    last_output
                        + chrono::Duration::seconds(
                            config.silent_interval.as_secs().saturating_mul(multiple) as i64,
                        ),
                    now,
                    watching,
                );
            }
            if let Some(range) = command.usual_ranges.get(&stage.name) {
                for multiple in [1.0, 2.0] {
                    deadline(
                        began.with_timezone(&Local)
                            + chrono::Duration::seconds(
                                (range.upper_seconds * multiple).floor() as i64 + 1,
                            ),
                        now,
                        watching,
                    );
                }
            }
        }
    }
    for child in command.children.iter().chain(&command.dispatched) {
        collect(child, Some(log), config, now, watching);
    }
}

fn deadline(at: DateTime<Local>, now: DateTime<Local>, watching: &mut Watching) {
    if let Ok(delay) = (at - now).to_std()
        && !delay.is_zero()
    {
        watching.deadline = Some(watching.deadline.map_or(delay, |old| old.min(delay)));
    }
}
