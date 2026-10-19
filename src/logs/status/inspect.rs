//! Render one Command and stream its selected log bytes.

use std::io::{self, Write};

use anyhow::{Context, Result};
use chrono::Local;
use serde::Serialize;

use crate::config::UserConfig;
use crate::logs::reading::{Command, Observation, Snapshot};

fn print(command: Observation<'_>) -> Result<()> {
    match command {
        Observation::Record(command) => print_details(command, "")?,
        Observation::History(command) => super::print_history(command),
    }
    if let Some(path) = command.command_log() {
        println!("Command log: {}", path.display());
    }
    if matches!(command, Observation::History(_)) {
        for path in command.sessions()? {
            println!("Session log: {}", path.display());
        }
    }
    Ok(())
}

fn print_details(command: &Command, indent: &str) -> Result<()> {
    println!(
        "{indent}{} {} · {}",
        command.record.kind,
        command.record.name,
        super::stage_text(command)
    );
    if let Some(issue) = &command.record.issue {
        println!(
            "{indent}{}",
            issue.title.as_deref().unwrap_or("(title unavailable)")
        );
    }
    for stage in &command.record.stages {
        println!("{indent}  {} · {}", stage.name, stage.began_at);
    }
    for path in command.sessions()? {
        println!("{indent}Session log: {}", path.display());
    }
    if let Some(event) = command.latest_event() {
        println!("{indent}{event}");
    }
    super::print_tickets(&command.record, &format!("{indent}  "));
    for child in command.dispatched.iter().chain(&command.children) {
        print_details(child, &format!("{indent}  "))?;
    }
    Ok(())
}

pub fn show(input: &str, json: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load(&config.logs_dir, Local::now())?;
    let command = snapshot.resolve(input)?.observation;
    if json {
        #[derive(Serialize)]
        struct Show<'a> {
            version: u32,
            hostname: String,
            command: crate::logs::reading::Detailed<'a>,
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&Show {
                version: 1,
                hostname: crate::host::name().unwrap_or_else(|| "unknown".into()),
                command: command.details()?,
            })?
        );
    } else {
        print(command)?;
    }
    Ok(())
}

pub fn logs(input: &str, session: Option<&str>, follow: bool) -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load(&config.logs_dir, Local::now())?;
    let selection = snapshot.resolve(input)?;
    let command = selection.observation;
    let path = match session {
        Some(kind) => command.session(kind)?,
        None => command
            .command_log()
            .context("this Command has no Command log")?
            .to_path_buf(),
    };
    let mut file =
        std::fs::File::open(&path).with_context(|| format!("could not open {}", path.display()))?;
    let mut stdout = io::stdout().lock();
    loop {
        // Check before draining, so a completed Command's final bytes are
        // always copied even when it ended between two polls.
        let ended = selection.process_exited();
        io::copy(&mut file, &mut stdout)?;
        stdout.flush()?;
        if !follow || ended {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
