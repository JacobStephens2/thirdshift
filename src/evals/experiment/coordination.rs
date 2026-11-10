//! Publish the Experiment before draining host work, so all new Passes skip.
use std::time::{Duration, Instant};

use anyhow::Result;

use super::store::Stored;
use crate::{interrupt, logs, poll, progress};

pub(super) fn start(experiment: &mut Stored, no_wait: bool) -> Result<()> {
    experiment.details.no_wait = no_wait;
    experiment.details.commands_at_start = if no_wait {
        logs::live_commands()?.len()
    } else {
        0
    };
    experiment.details.waiting_for.clear();
    experiment.save()?;
    if no_wait {
        progress::step(experiment.details.contention().unwrap());
        return Ok(());
    }
    let mut previous = Vec::new();
    let mut last_line = Instant::now();
    loop {
        interrupt::check()?;
        let mut waiting: Vec<_> = logs::live_commands()?
            .into_iter()
            .map(|command| command.name)
            .collect();
        if waiting.is_empty() && crate::host_work::busy()? {
            waiting.push("Commands starting or ending on this machine".into());
        }
        waiting.sort();
        if waiting.is_empty() {
            logs::waiting_for(&[]);
            experiment.details.waiting_for.clear();
            experiment.save()?;
            return Ok(());
        }
        if waiting != previous || last_line.elapsed() >= Duration::from_secs(30) {
            logs::stage(logs::Stage::WaitingForCommands);
            progress::step(format!("Waiting for Commands: {}", waiting.join(", ")));
            logs::waiting_for(&waiting);
            experiment.details.waiting_for = waiting.clone();
            experiment.save()?;
            previous = waiting;
            last_line = Instant::now();
        }
        poll::sleep(Duration::from_secs(1))?;
    }
}
