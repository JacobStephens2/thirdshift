//! Session log recording: consume the transcript, checkpoint stream Models,
//! and replace them only when completed Interpretation supplies a report.
//! The captured owning Command also governs child inheritance and collection.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::harness::interpretation::{Ended, Interpretation};
use crate::harness::{Choice, process};
use crate::logs::owner::Owner;
use crate::{logs, progress};

/// Immutable Command identity, captured after Command logging is established.
#[derive(Clone)]
pub struct Recording {
    /// A Trial explicitly has no Command owner. Allocation failures in a
    /// Command remain errors; they must never select the Trial's storage.
    owner: std::result::Result<Option<Owner>, String>,
}

/// The Session prompt, its transport, and execution limits for private Graders.
#[derive(Default)]
pub struct Input<'a> {
    pub prompt: &'a str,
    pub stdin: Option<&'a str>,
    pub deadline: Option<Instant>,
    /// Keep decoded progress and completion text out of factory Command logs.
    pub private: bool,
}

impl Recording {
    /// Capture current logging facts once, preferring a local Command log to
    /// inherited ownership. Child Runs retain their parent's exact path.
    pub fn capture() -> Self {
        Self {
            owner: logs::captured_owner().map(Some),
        }
    }

    /// Trial artifacts have their own empty output directory and no Command.
    pub fn for_trial() -> Self {
        Self { owner: Ok(None) }
    }

    /// Construct from captured facts; fixtures need no process-global changes.
    #[cfg(test)]
    pub fn new(local: Option<PathBuf>, inherited: Option<PathBuf>, stamp: String) -> Self {
        Self {
            owner: local
                .or(inherited)
                .map(|log| Owner::fixture(Some(log), stamp))
                .map(Some)
                .ok_or_else(|| "no owner in fixture".into()),
        }
    }

    pub fn session_path(&self, dir: &Path, subject: &str, kind: &str) -> Result<PathBuf> {
        let owner = self
            .owner
            .as_ref()
            .map_err(|cause| anyhow::anyhow!("cannot name Session log: {cause}"))?;
        Ok(match owner {
            Some(owner) => owner.session_path(dir, subject, kind),
            None => dir.join(format!("{subject}-{kind}.jsonl")),
        })
    }

    fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref().ok()?.as_ref()
    }

    fn command_log(&self) -> Option<&Path> {
        self.owner()?.log_path()
    }

    /// Give a child this exact owning Command, including through nested Runs.
    /// Absence removes stale ownership from the child's environment.
    pub fn inherit_command(&self, command: &mut Command) {
        Owner::inherit(self.owner(), command);
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
        input: Input<'_>,
        log: &Path,
        stream: Interpretation,
    ) -> Result<Ended> {
        let adapter = choice.harness.adapter();
        if matches!(self.owner, Ok(None)) {
            // A Trial must not lend a caller's stale Command identity to
            // the Harness or to thirdshift processes it might start.
            self.inherit_command(command);
        }
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        let mut log_file =
            File::create(log).with_context(|| format!("could not create {}", log.display()))?;
        let started = Instant::now();
        let context = logs::trace_context()
            .or_else(|| self.owner().map(logs::trace::Context::of_owner))
            .unwrap_or_else(|| logs::trace::Context::of_session(log));
        let trace = Arc::new(Mutex::new(super::trace::Trace::create(
            log,
            kind,
            choice,
            input.prompt,
            context,
            command,
            self.command_log(),
        )?));
        let streaming_trace = Arc::clone(&trace);
        let (kind_owned, log_owned) = (kind.to_string(), log.to_path_buf());
        let choice_owned = choice.clone();
        let recording = self.clone();
        let private = input.private;
        let executed = process::streaming(
            adapter,
            command,
            input.stdin,
            input.deadline,
            stream,
            move |output, stream| {
                let followed = follow(
                    &kind_owned,
                    output,
                    &mut log_file,
                    &log_owned,
                    stream,
                    &streaming_trace,
                    private,
                );
                // The process owner joins this reader before returning, including
                // on interruption, when it deliberately suppresses state recovery.
                recording.keep(
                    &kind_owned,
                    &log_owned,
                    &choice_owned,
                    stream.observed_models(),
                );
                followed
            },
        );

        let executed = match executed {
            Ok(executed) => executed,
            Err(error) => {
                let finished = trace.lock().unwrap().finish(Some(&format!("{error:#}")));
                warn_unfinished_trace(finished);
                return Err(error);
            }
        };

        let completion = executed.state.finish(executed.execution);
        let finished = (|| {
            let mut trace = trace.lock().unwrap();
            if let Some(report) = &completion.report {
                for record in &report.retained_records {
                    trace.record(&record.raw, &record.events)?;
                }
                trace.reconcile(&report.events)?;
            }
            trace.finish(
                completion
                    .outcome
                    .as_ref()
                    .err()
                    .map(|error| error.to_string())
                    .as_deref(),
            )
        })();
        warn_unfinished_trace(finished);
        if let Some(report) = completion.report {
            self.keep(kind, log, choice, report.models.clone());
            if private {
                return completion.outcome;
            }
            for line in report.completion_progress {
                progress::step(format_args!("{kind}: {line}"));
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
            command_log: self.command_log().map(Path::to_path_buf),
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

    /// Read this Command's evidence, including child Runs sharing its owner.
    /// Other commands in the same repository and folder are excluded.
    fn collect(&self) -> Result<Vec<String>> {
        let Some(command_log) = self.command_log() else {
            return Ok(Vec::new());
        };
        let Some(dir) = command_log.parent() else {
            return Ok(Vec::new());
        };
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if self.owner().is_some_and(|owner| owner.models_match(&path)) {
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
            if record.command_log.as_deref() != Some(command_log) {
                continue;
            }
            let models = if record.observed.is_empty() {
                record.requested.map_or_else(
                    || "Harness default (actual model not reported)".to_string(),
                    |model| format!("{model} (requested; actual model not reported)"),
                )
            } else {
                let mut names = Vec::new();
                for model in record.observed {
                    if !names.contains(&model) {
                        names.push(model);
                    }
                }
                format!("{} (reported)", names.join(", "))
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
    trace: &Mutex<super::trace::Trace>,
    private: bool,
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
        let raw = String::from_utf8_lossy(&line);
        let decoded = progress.decode(&raw);
        trace.lock().unwrap().record(&raw, &decoded.events)?;
        if !private {
            for condensed in decoded.progress {
                progress::step(format_args!("{kind}: {condensed}"));
            }
        }
    }
}

/// A Session trace that cannot be finished costs evidence, never the session.
fn warn_unfinished_trace(finished: std::io::Result<()>) {
    if let Err(error) = finished {
        progress::step(format_args!(
            "warning: could not finish the session trace: {error:#}"
        ));
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
