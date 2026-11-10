//! Same-account, same-machine Commands, counted once through their outer tree.
use anyhow::Result;
use chrono::Local;

use super::reading::{Entry, Snapshot};

pub(crate) struct Command {
    pub name: String,
    pub experiment: Option<String>,
}

pub(super) fn read(root: &std::path::Path) -> Result<Vec<Command>> {
    let snapshot = Snapshot::load(root, Local::now())?;
    let host = crate::host::name().unwrap_or_else(|| "unknown".into());
    Ok(snapshot
        .entries()
        .iter()
        .filter_map(|entry| {
            let Entry::Record(command) = entry else {
                return None;
            };
            let record = &command.record;
            if record.hostname != host
                || record.process_id == std::process::id()
                || super::reading::active_runs(std::slice::from_ref(entry)) == 0
            {
                return None;
            }
            Some(Command {
                name: format!("{} {}", super::display_name(&record.kind), record.name),
                experiment: (record.kind == "Experiment").then(|| {
                    record
                        .experiment
                        .as_ref()
                        .map_or_else(|| record.name.clone(), |experiment| experiment.id.clone())
                }),
            })
        })
        .collect())
}
