//! Session log recording: consume the transcript, checkpoint stream Models,
//! and replace them only when completed Interpretation supplies a report.
//! The captured owning Command also governs child inheritance and collection.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::harness::interpretation::{Ended, Interpretation};
use crate::harness::{Choice, process};
use crate::{logs, progress};

const PARENT_COMMAND_LOG: &str = "THIRDSHIFT_PARENT_COMMAND_LOG";

/// Immutable Command identity, captured after Command logging is established.
#[derive(Clone)]
pub struct Recording {
    command_log: Option<PathBuf>,
    stamp: String,
}

impl Recording {
    /// Capture current logging facts once, preferring a local Command log to
    /// inherited ownership. Child Runs retain their parent's exact path.
    pub fn capture() -> Self {
        Self::new(
            logs::command_log_path(),
            std::env::var_os(PARENT_COMMAND_LOG).map(PathBuf::from),
            logs::stamp(),
        )
    }

    /// Construct from captured facts; fixtures need no process-global changes.
    pub fn new(local: Option<PathBuf>, inherited: Option<PathBuf>, stamp: String) -> Self {
        Self {
            command_log: local.or(inherited),
            stamp,
        }
    }

    /// Give a child this exact owning Command, including through nested Runs.
    /// Absence removes stale ownership from the child's environment.
    pub fn inherit_command(&self, command: &mut Command) {
        if let Some(log) = &self.command_log {
            command.env(PARENT_COMMAND_LOG, log);
        } else {
            command.env_remove(PARENT_COMMAND_LOG);
        }
    }

    /// Collection failures cost attribution, never notification delivery.
    pub fn notification_lines(&self) -> Vec<String> {
        match self.collect() {
            Ok(lines) => lines,
            Err(error) => {
                progress::step(format_args!(
                    "warning: could not read session models for the Run notification: {error:#}"
                ));
                Vec::new()
            }
        }
    }

    /// Run one Session with the selected launch command and Interpretation.
    /// Preserve stream evidence before supervision can discard recovered state;
    /// a present completion report replaces it, even with no observed Models.
    /// Evidence writes are warnings and never change the Session outcome.
    pub fn run(
        &self,
        kind: &str,
        choice: &Choice,
        command: &mut Command,
        input: Option<&str>,
        log: &Path,
        stream: Interpretation,
    ) -> Result<Ended> {
        let adapter = choice.harness.adapter();
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        let mut log_file =
            File::create(log).with_context(|| format!("could not create {}", log.display()))?;
        let started = Instant::now();
        let (kind_owned, log_owned) = (kind.to_string(), log.to_path_buf());
        let choice_owned = choice.clone();
        let recording = self.clone();
        let executed =
            process::streaming(adapter, command, input, stream, move |output, stream| {
                let followed = follow(&kind_owned, output, &mut log_file, &log_owned, stream);
                // The process owner joins this reader before returning, including
                // on interruption, when it deliberately suppresses state recovery.
                recording.keep(
                    &kind_owned,
                    &log_owned,
                    &choice_owned,
                    stream.observed_models(),
                );
                followed
            })?;

        let completion = executed.state.finish(executed.execution);
        if let Some(report) = completion.report {
            self.keep(kind, log, choice, report.models.clone());
            // Stream models already have security progress lines. Retained records
            // need theirs here, as before.
            if matches!(
                choice.harness,
                crate::harness::Harness::Muse | crate::harness::Harness::OpenCode
            ) {
                for model in report.models {
                    progress::step(format_args!("{kind}: Model: {model}"));
                }
            }
            for warning in report.warnings {
                progress::step(format_args!("{kind}: {warning}"));
            }
            let elapsed = minutes_and_seconds(started.elapsed());
            let summary = report
                .summary
                .map_or(String::new(), |summary| format!(": {summary}"));
            progress::step(format_args!(
                "{kind}: session ended after {elapsed}{summary}"
            ));
        }
        completion.outcome
    }

    /// Record only model metadata; failure costs attribution, never the session.
    fn keep(&self, kind: &str, log: &Path, choice: &Choice, observed: Vec<String>) {
        let record = SessionModels {
            command_log: self.command_log.clone(),
            kind: kind.to_string(),
            harness: choice.harness.name().to_string(),
            requested: choice.model.clone(),
            effort: choice.effort.clone(),
            observed,
        };
        let path = log.with_extension("models.json");
        let kept = serde_json::to_vec(&record)
            .map_err(anyhow::Error::from)
            .and_then(|json| fs::write(&path, json).map_err(anyhow::Error::from));
        if let Err(error) = kept {
            progress::step(format_args!(
                "warning: could not record session models: {error:#}"
            ));
        }
    }

    /// Read this command's evidence, including child Runs sharing its start stamp.
    /// Other commands in the same repository and folder are excluded.
    fn collect(&self) -> Result<Vec<String>> {
        let Some(command_log) = &self.command_log else {
            return Ok(Vec::new());
        };
        let Some(dir) = command_log.parent() else {
            return Ok(Vec::new());
        };
        let marker = format!("-{}-", self.stamp);
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(&marker) && name.ends_with(".models.json"))
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut lines = Vec::new();
        for path in paths {
            let read = fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|json| {
                    serde_json::from_slice::<SessionModels>(&json).map_err(anyhow::Error::from)
                })
                .with_context(|| format!("could not read {}", path.display()));
            let record = match read {
                Ok(record) => record,
                Err(error) => {
                    progress::step(format_args!(
                        "warning: could not read session models for the Run notification: {error:#}"
                    ));
                    continue;
                }
            };
            if record.command_log.as_deref() != Some(command_log.as_path()) {
                continue;
            }
            let models = if record.observed.is_empty() {
                record.requested.map_or_else(
                    || "Harness default (not reported)".to_string(),
                    |model| format!("{model} (requested)"),
                )
            } else {
                let mut names = Vec::new();
                for model in record.observed {
                    if !names.contains(&model) {
                        names.push(model);
                    }
                }
                names.join(", ")
            };
            lines.push(format!(
                "- {}: {} · {models} · session effort: {}",
                record.kind,
                record.harness,
                record.effort.as_deref().unwrap_or("default effort")
            ));
        }
        Ok(lines)
    }
}

#[derive(Serialize, Deserialize)]
struct SessionModels {
    command_log: Option<PathBuf>,
    kind: String,
    harness: String,
    requested: Option<String>,
    effort: Option<String>,
    observed: Vec<String>,
}

/// Copy every line of `stream` to `log_file` and print the progress lines it
/// condenses to, until the stream ends.
fn follow(
    kind: &str,
    stream: impl Read,
    log_file: &mut File,
    log: &Path,
    progress: &mut Interpretation,
) -> Result<()> {
    let mut stream = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        if stream
            .read_until(b'\n', &mut line)
            .context("could not read the session stream")?
            == 0
        {
            return Ok(());
        }
        log_file
            .write_all(&line)
            .with_context(|| format!("could not write {}", log.display()))?;
        for condensed in progress.condense(&String::from_utf8_lossy(&line)) {
            progress::step(format_args!("{kind}: {condensed}"));
        }
    }
}

/// `5m 32s`, or `8s` under a minute.
fn minutes_and_seconds(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds / 60 {
        0 => format!("{seconds}s"),
        minutes => format!("{minutes}m {}s", seconds % 60),
    }
}

#[cfg(test)]
mod tests;
