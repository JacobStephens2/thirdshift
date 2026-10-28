//! Atomic metadata and one result per Trial. An exclusive lock fences resumes;
//! a short metadata lock fences each `experiment.json` write, renames included.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::trial_results::files::{Files, Native};
use super::{Arm, Description, Task, TrialResult, trial_results};
use crate::trial::{Outcome, VoidCause};

#[derive(Serialize, Deserialize)]
pub(super) struct Stored {
    format: u32,
    #[serde(flatten)]
    pub description: Description,
    pub state: State,
    #[serde(flatten)]
    pub details: Details,
    pub(super) created_at: String,
    /// The Day shift's name; unset means the default from the Arms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub verdict: Option<super::grading::Verdict>,
    #[serde(default)]
    pub rechecks: super::rechecks::History,
    #[serde(default)]
    pub pending_rechecks: Vec<super::grading::Pending>,
    #[serde(skip)]
    pub directory: PathBuf,
    #[serde(skip)]
    lock: Option<File>,
}

#[derive(Serialize)]
pub(crate) struct View {
    pub format: u32,
    pub name: String,
    pub name_set: bool,
    #[serde(flatten)]
    pub description: Description,
    pub state: State,
    #[serde(flatten)]
    pub details: Details,
    pub created_at: String,
    pub tasks: Vec<TaskName>,
    pub running_trials: Vec<super::RunningTrial>,
    pub sessions: Vec<super::SessionLog>,
    pub results: Vec<TrialResult>,
    pub unfinished_trials: Vec<TrialResult>,
    pub verdict: Option<super::grading::Verdict>,
    pub pending_rechecks: Vec<super::grading::Pending>,
    pub minutes: Minutes,
    pub usage: super::usage::Arms,
    pub voids: Voids,
    pub findings: Vec<String>,
}

/// Facts for readers; these never participate in Experiment identity or the Bar.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Details {
    #[serde(default)]
    pub baseline_ref: Option<String>,
    #[serde(default)]
    pub candidate_ref: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub pause_reason: Option<String>,
    #[serde(default)]
    pub resume_command: Option<String>,
    #[serde(default)]
    pub command_log: Option<PathBuf>,
    #[serde(default)]
    pub release: bool,
}

#[derive(Serialize)]
pub(crate) struct TaskName {
    pub suite: String,
    pub name: String,
}

#[derive(Default, Serialize)]
pub(crate) struct Voids {
    pub baseline: ArmVoids,
    pub candidate: ArmVoids,
}

#[derive(Default, Serialize)]
pub(crate) struct ArmVoids {
    pub count: usize,
    pub causes: BTreeMap<VoidCause, usize>,
}

impl Voids {
    pub(super) fn include(&mut self, result: &TrialResult) {
        let arm = match result.arm {
            Arm::Baseline => &mut self.baseline,
            Arm::Candidate => &mut self.candidate,
        };
        for attempt in &result.attempts {
            if attempt.outcome == Outcome::Void {
                arm.count += 1;
                if let Some(cause) = attempt.void_cause {
                    *arm.causes.entry(cause).or_default() += 1;
                }
            }
        }
    }

    pub(super) fn findings(&self) -> Vec<String> {
        [("Baseline", &self.baseline), ("Candidate", &self.candidate)]
            .into_iter()
            .filter_map(|(name, arm)| {
                let count = arm.causes.get(&VoidCause::MemoryCap).copied().unwrap_or(0);
                (count > 1).then(|| {
                    format!("{name} Arm repeatedly hit its memory cap ({count} Void Trials)")
                })
            })
            .collect()
    }
}

impl std::fmt::Display for ArmVoids {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} Void Trials", self.count)?;
        if !self.causes.is_empty() {
            let causes = self
                .causes
                .iter()
                .map(|(cause, count)| format!("{cause}: {count}"))
                .collect::<Vec<_>>();
            write!(formatter, " ({})", causes.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Default, Serialize)]
pub(crate) struct Minutes {
    pub baseline: f64,
    pub candidate: f64,
}

impl Minutes {
    pub(super) fn include(&mut self, result: &TrialResult) {
        let arm = match result.arm {
            Arm::Baseline => &mut self.baseline,
            Arm::Candidate => &mut self.candidate,
        };
        *arm += result
            .attempts
            .iter()
            .map(|attempt| attempt.timings.total_ms as f64 / 60_000.0)
            .sum::<f64>();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Prepared,
    Running,
    Completed,
    Interrupted,
    Paused,
}

impl std::fmt::Display for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Interrupted => "interrupted",
            Self::Paused => "paused",
        })
    }
}

impl Stored {
    pub fn open(
        root: &Path,
        mut description: Description,
        tasks: impl FnOnce(&Description) -> Result<Option<Vec<Task>>>,
    ) -> Result<Option<Self>> {
        fs::create_dir_all(root)?;
        let directory = root.join(&description.id);
        fs::create_dir_all(&directory)?;
        let lock = File::create(directory.join("lock"))?;
        lock.try_lock().context("Experiment is already running")?;
        let path = directory.join("experiment.json");
        let mut stored = if path.exists() {
            let mut stored: Self = serde_json::from_slice(&fs::read(path)?)?;
            ensure!(stored.format == 1, "unsupported Experiment format");
            let started = directory.join("started").exists();
            if started {
                // Cause defaults belong to the installed runner, not a CLI
                // override. Resume keeps the rules recorded before its Trials.
                description
                    .bar
                    .void_rules
                    .causes
                    .clone_from(&stored.description.bar.void_rules.causes);
            }
            if stored.description.bar != description.bar {
                ensure!(!started, "Bar cannot change after the first Trial");
                stored.description.bar = description.bar;
            }
            stored
        } else {
            // Selection may skip a new release Experiment. Keep it out of
            // saved results while retaining the exclusive creation lock.
            let Some(tasks) = tasks(&description)? else {
                return Ok(None);
            };
            Self {
                format: 1,
                description,
                state: State::Prepared,
                details: Details::default(),
                created_at: chrono::Local::now().to_rfc3339(),
                name: None,
                tasks,
                verdict: None,
                rechecks: BTreeMap::new(),
                pending_rechecks: Vec::new(),
                directory: PathBuf::new(),
                lock: None,
            }
        };
        stored.directory = directory;
        stored.lock = Some(lock);
        stored.upgrade_verdict()?;
        stored.save()?;
        Ok(Some(stored))
    }

    /// Remove an Experiment this command created and nothing has run in.
    pub fn discard(self) -> Result<()> {
        fs::remove_dir_all(&self.directory)?;
        Ok(())
    }

    /// A rename may land while a run holds the Experiment lock, so every
    /// metadata write keeps the name saved on disk.
    pub fn save(&mut self) -> Result<()> {
        let _metadata = metadata_lock(&self.directory)?;
        let path = self.directory.join("experiment.json");
        if path.exists() {
            #[derive(Deserialize)]
            struct Saved {
                #[serde(default)]
                name: Option<String>,
            }
            let saved: Saved = serde_json::from_slice(&fs::read(&path)?)?;
            self.name = saved.name;
        }
        write(&path, self)
    }

    pub fn rename(&mut self, name: Option<String>) -> Result<()> {
        let _metadata = metadata_lock(&self.directory)?;
        self.name = name;
        write(&self.directory.join("experiment.json"), self)
    }

    pub fn name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| super::names::default(&self.description, &self.details))
    }

    pub fn results(&self) -> Result<Vec<TrialResult>> {
        Ok(trial_results::effective(
            trial_results::terminal(&self.directory)?,
            &self.rechecks,
        ))
    }

    pub fn refresh_verdict(&mut self) -> Result<()> {
        let (verdict, pending) =
            super::grading::assess(&self.tasks, &self.results()?, &self.description.bar)?;
        self.verdict = Some(verdict);
        self.pending_rechecks = pending;
        Ok(())
    }

    fn upgrade_verdict(&mut self) -> Result<()> {
        if self.state == State::Completed
            && self
                .verdict
                .as_ref()
                .is_none_or(|verdict| verdict.provisional_verdict.is_none())
        {
            self.refresh_verdict()?;
        }
        Ok(())
    }

    fn directory(root: &Path, id: &str) -> Result<PathBuf> {
        ensure!(
            id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid Experiment ID"
        );
        let directory = root.join(id);
        ensure!(directory.is_dir(), "Experiment not found");
        Ok(directory)
    }

    /// Validate saved metadata without refreshing a legacy Verdict or reading
    /// any Trial artifacts. Evidence readers share ordinary load validation.
    pub(super) fn retained(files: &impl Files, root: &Path, id: &str) -> Result<Self> {
        Self::metadata(files, Self::directory(root, id)?, id)
    }

    fn metadata(files: &impl Files, directory: PathBuf, id: &str) -> Result<Self> {
        let mut stored: Self = serde_json::from_slice(
            &files
                .read(&directory.join("experiment.json"))
                .context("Experiment not found")?,
        )?;
        ensure!(stored.format == 1, "unsupported Experiment format");
        ensure!(
            stored.description.id == id,
            "Experiment ID does not match its folder"
        );
        stored.directory = directory;
        Ok(stored)
    }

    pub fn load(root: &Path, id: &str, exclusive: bool) -> Result<Self> {
        let directory = Self::directory(root, id)?;
        let lock = if exclusive {
            let lock = File::create(directory.join("lock"))?;
            lock.try_lock()
                .context("Experiment is already running or being rechecked")?;
            Some(lock)
        } else {
            None
        };
        let mut stored = Self::metadata(&Native, directory, id)?;
        stored.lock = lock;
        stored.upgrade_verdict()?;
        Ok(stored)
    }

    fn view(&self) -> Result<View> {
        super::reading::read(self)
    }

    pub fn render(&self, json: bool) -> Result<String> {
        let view = self.view()?;
        if json {
            Ok(serde_json::to_string(&view)?)
        } else {
            Ok(format!(
                "{} · {} · {} · {} → {} · {} Trial results · Verdict: {} · Baseline: {:.3} minutes{} · Candidate: {:.3} minutes{}\nBaseline: {}\nCandidate: {}{}",
                self.description.id,
                view.name,
                self.state,
                self.description.baseline.commit,
                self.description.candidate.commit,
                view.results.len(),
                self.verdict
                    .as_ref()
                    .map_or("pending", |verdict| verdict.outcome())
                    .replace('_', " "),
                view.minutes.baseline,
                view.usage.baseline(),
                view.minutes.candidate,
                view.usage.candidate(),
                view.voids.baseline,
                view.voids.candidate,
                view.findings
                    .iter()
                    .map(|finding| format!("\nFinding: {finding}"))
                    .chain(
                        view.pending_rechecks
                            .iter()
                            .map(|pending| format!("\nRecheck: {pending}"))
                    )
                    .collect::<String>()
            ))
        }
    }
}

/// Held only across one read-and-write of `experiment.json`, never for a run.
fn metadata_lock(directory: &Path) -> Result<File> {
    let lock = File::create(directory.join("metadata.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("missing Experiment folder")?)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.flush()?;
    temporary.persist(path)?;
    Ok(())
}

pub(super) fn list(root: &Path) -> Result<Vec<Stored>> {
    let mut experiments = Vec::new();
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let path = entry?.path().join("experiment.json");
            if path.is_file() {
                let directory = path.parent().unwrap();
                let id = directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .context("invalid Experiment ID")?;
                experiments.push(Stored::load(root, id, false)?);
            }
        }
    }
    // Retained timestamps include local offsets, so lexical order is not time order.
    experiments.sort_by(|a, b| {
        chrono::DateTime::parse_from_rfc3339(&b.created_at)
            .ok()
            .cmp(&chrono::DateTime::parse_from_rfc3339(&a.created_at).ok())
            .then_with(|| a.description.id.cmp(&b.description.id))
    });
    Ok(experiments)
}

pub(crate) fn read(id: Option<&str>) -> Result<Vec<View>> {
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    if let Some(id) = id {
        return Ok(vec![Stored::load(&root, id, false)?.view()?]);
    }
    list(&root)?.iter().map(Stored::view).collect()
}

/// Renames in any state: only the metadata lock fences this write, as it does
/// a running Experiment's own.
pub(super) fn rename(root: &Path, id: &str, name: Option<String>, json: bool) -> Result<String> {
    let directory = Stored::directory(root, id)?;
    let mut stored = {
        let _metadata = metadata_lock(&directory)?;
        let mut stored = Stored::metadata(&Native, directory, id)?;
        stored.name = name;
        write(&stored.directory.join("experiment.json"), &stored)?;
        stored
    };
    stored.upgrade_verdict()?;
    stored.render(json)
}

pub(super) fn show(root: &Path, id: Option<&str>, json: bool) -> Result<String> {
    if let Some(id) = id {
        return Stored::load(root, id, false)?.render(json);
    }
    let experiments = list(root)?;
    if json {
        let views = experiments
            .iter()
            .map(Stored::view)
            .collect::<Result<Vec<_>>>()?;
        Ok(serde_json::to_string(&views)?)
    } else {
        experiments
            .iter()
            .map(|experiment| experiment.render(false))
            .collect::<Result<Vec<_>>>()
            .map(|lines| lines.join("\n"))
    }
}
