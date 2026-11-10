//! Same-account, same-machine Commands, counted once through their outer tree.
use anyhow::Result;
use chrono::Local;

pub(crate) struct Command {
    pub name: String,
    pub experiment: Option<String>,
    pub process_id: u32,
    pub waiting: bool,
}

pub(super) fn read(root: &std::path::Path) -> Result<Vec<Command>> {
    let commands = super::reading::live_commands(root, Local::now())?;
    let host = crate::host::name().unwrap_or_else(|| "unknown".into());
    Ok(commands
        .iter()
        .filter_map(|command| {
            let record = &command.record;
            if record.hostname != host || record.process_id == std::process::id() {
                return None;
            }
            Some(Command {
                name: format!("{} {}", super::display_name(&record.kind), record.name),
                process_id: record.process_id,
                waiting: !record.waiting_for.is_empty(),
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
