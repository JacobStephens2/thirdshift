//! Publish the Experiment before draining host work, so all new Passes skip.
use std::time::{Duration, Instant};

use anyhow::Result;

use super::store::Stored;
use crate::{interrupt, logs, poll, progress};

pub(super) fn waiting_for(experiment: &mut Stored, commands: &[String]) {
    logs::waiting_for(commands);
    experiment.details.waiting_for = commands.to_vec();
}

pub(super) fn start(experiment: &mut Stored, no_wait: bool) -> Result<()> {
    experiment.details.no_wait = no_wait;
    experiment.details.commands_at_start = if no_wait {
        logs::live_commands()?.len()
    } else {
        0
    };
    waiting_for(experiment, &[]);
    experiment.save()?;
    if no_wait {
        progress::step(experiment.details.contention().unwrap());
        return Ok(());
    }
    let mut previous = Vec::new();
    let mut last_line = Instant::now();
    loop {
        interrupt::check()?;
        let mut commands = logs::live_commands()?;
        let busy = crate::host_work::busy()?;
        // Waiting Experiments must not wait on one another forever. Once
        // ordinary work has drained, a stable order admits exactly one; the
        // others keep waiting until its Trials and Graders have finished.
        if !busy
            && commands.iter().all(|command| {
                command.experiment.is_some()
                    && command.waiting
                    && std::process::id() < command.process_id
            })
        {
            commands.clear();
        }
        let mut waiting: Vec<_> = commands.into_iter().map(|command| command.name).collect();
        if waiting.is_empty() && busy {
            waiting.push("Commands starting or ending on this machine".into());
        }
        waiting.sort();
        if waiting.is_empty() {
            waiting_for(experiment, &[]);
            experiment.save()?;
            return Ok(());
        }
        if waiting != previous || last_line.elapsed() >= Duration::from_secs(30) {
            logs::stage(logs::Stage::WaitingForCommands);
            progress::step(format!("Waiting for Commands: {}", waiting.join(", ")));
            waiting_for(experiment, &waiting);
            experiment.save()?;
            previous = waiting;
            last_line = Instant::now();
        }
        poll::sleep(Duration::from_secs(1))?;
    }
}
