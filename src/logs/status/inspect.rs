//! Render one Command and stream its selected log bytes.

use std::io::{self, Write};

use anyhow::{Context, Result};
use chrono::Local;
use serde::Serialize;

use crate::config::UserConfig;
use crate::logs::reading::{Command, Observation, Snapshot};

/// Exactly the detailed observations used by show, for the local Console.
pub(crate) fn details(config: &UserConfig, input: &str) -> Result<serde_json::Value> {
    let snapshot =
        Snapshot::load_with_silence(&config.logs_dir, Local::now(), config.silent_interval)?;
    Ok(serde_json::to_value(
        snapshot.resolve(input)?.observation.details()?,
    )?)
}

pub(crate) fn log_path(
    config: &UserConfig,
    input: &str,
    session: Option<&str>,
) -> Result<std::path::PathBuf> {
    let snapshot = Snapshot::load(&config.logs_dir, Local::now())?;
    let command = snapshot.resolve(input)?.observation;
    selected_log(command, session)
}

fn selected_log(command: Observation<'_>, session: Option<&str>) -> Result<std::path::PathBuf> {
    match session {
        Some(session) => command.session(session),
        None => command
            .command_log()
            .context("this Command has no Command log")
            .map(std::path::Path::to_path_buf),
    }
}

pub(crate) fn url(route: &str, command: &str, session: Option<&std::path::Path>) -> String {
    let mut url = reqwest::Url::parse("http://localhost").expect("fixed URL");
    url.set_path(route);
    url.query_pairs_mut().append_pair("command", command);
    if let Some(session) = session {
        url.query_pairs_mut()
            .append_pair("session", &session.to_string_lossy());
    }
    format!("{}?{}", url.path(), url.query().unwrap_or_default())
}

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
        let usual = command
            .usual_ranges
            .get(&stage.name)
            .map(|range| format!(" · usual {}", range.text()))
            .unwrap_or_default();
        println!("{indent}  {} · {}{usual}", stage.name, stage.began_at);
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
    let snapshot =
        Snapshot::load_with_silence(&config.logs_dir, Local::now(), config.silent_interval)?;
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
    let path = selected_log(command, session)?;
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
