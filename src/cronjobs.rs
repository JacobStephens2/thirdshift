//! Read this Unix account's thirdshift entries without changing its crontab.
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Serialize)]
pub struct CronJob {
    pub schedule: String,
    pub command: String,
}

pub fn load() -> Result<Vec<CronJob>> {
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
    let jobs = load()?;
    if json {
        return Ok(serde_json::to_string_pretty(&jobs)?);
    }
    if jobs.is_empty() {
        return Ok("No thirdshift cron jobs in this account's crontab.".into());
    }
    Ok(jobs
        .iter()
        .map(|job| format!("{}  {}", job.schedule, job.command))
        .collect::<Vec<_>>()
        .join("\n"))
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
    // Match literal executable names, including quoted paths and commands
    // after cd, env or flock. Never evaluate the shell text to discover jobs.
    let mentions_thirdshift = command
        .split(|c: char| c.is_whitespace() || "'\";&|()<>".contains(c))
        .any(|word| word.rsplit('/').next() == Some("thirdshift"));
    mentions_thirdshift.then(|| CronJob {
        schedule: schedule.join(" "),
        command: command.into(),
    })
}
