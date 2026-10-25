//! Read this Unix account's thirdshift entries without changing its crontab.
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::config::UserConfig;
use crate::logs::pass_starts::{self, PassStart};

#[derive(Serialize)]
pub(crate) struct CronJobs {
    pub jobs: Vec<CronJob>,
    pub pass_starts: Vec<PassStart>,
}

#[derive(Serialize)]
pub struct CronJob {
    pub schedule: String,
    pub command: String,
}

pub(crate) fn load() -> Result<CronJobs> {
    let config = UserConfig::load()?;
    Ok(CronJobs {
        jobs: jobs()?,
        pass_starts: pass_starts::load(&config.logs_dir)?,
    })
}

fn jobs() -> Result<Vec<CronJob>> {
    let output = Command::new("crontab")
        .arg("-l")
        .env("LC_ALL", "C")
        .output()
        .context("can't read cron jobs: could not run crontab -l")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        let diagnostic = error
            .trim()
            .strip_prefix("crontab: ")
            .unwrap_or(error.trim());
        if output.status.code() == Some(1) && diagnostic.starts_with("no crontab for ") {
            return Ok(Vec::new());
        }
        bail!(
            "can't read cron jobs (crontab -l {}): {}",
            output.status,
            error.trim()
        );
    }
    let crontab = String::from_utf8(output.stdout).context("crontab is not UTF-8")?;
    Ok(crontab.lines().filter_map(entry).collect())
}

pub fn show(json: bool) -> Result<String> {
    let inventory = load()?;
    if json {
        return Ok(serde_json::to_string_pretty(&inventory)?);
    }
    let mut lines = Vec::new();
    if inventory.jobs.is_empty() {
        lines.push("No thirdshift cron jobs in this account's crontab.".into());
    }
    lines.extend(
        inventory
            .jobs
            .iter()
            .map(|job| format!("{}  {}", job.schedule, job.command)),
    );
    lines.extend(inventory.pass_starts.iter().map(PassStart::line));
    if inventory.pass_starts.is_empty() {
        lines.push("No Pass start times recorded on this host.".into());
    }
    Ok(lines.join("\n"))
}

fn entry(line: &str) -> Option<CronJob> {
    let mut command = line.trim();
    // User crontabs have five schedule fields, or one @-schedule field.
    // Requiring a schedule excludes comments and environment assignments.
    let fields = if command.starts_with('@') {
        1
    } else if command.starts_with(|c: char| c.is_ascii_digit() || c == '*') {
        5
    } else {
        return None;
    };
    let mut schedule = Vec::new();
    for _ in 0..fields {
        let end = command.find(char::is_whitespace)?;
        schedule.push(&command[..end]);
        command = command[end..].trim_start();
    }
    // Cron sends everything after its first unescaped % to stdin, even inside
    // shell quotes. Keep it for display, but don't match input as a command.
    let mut escaped = false;
    let command_end = command
        .char_indices()
        .find_map(|(index, character)| {
            let separator = character == '%' && !escaped;
            escaped = character == '\\' && !escaped;
            separator.then_some(index)
        })
        .unwrap_or(command.len());
    // Match literal executable names, including quoted paths and commands
    // after cd, env or flock. Never evaluate the shell text to discover jobs.
    let mentions_thirdshift = command[..command_end]
        .split(|c: char| c.is_whitespace() || "'\";&|()<>".contains(c))
        .any(|word| word.rsplit('/').next() == Some("thirdshift"));
    mentions_thirdshift.then(|| CronJob {
        schedule: schedule.join(" "),
        command: command.into(),
    })
}
