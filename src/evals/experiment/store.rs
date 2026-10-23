//! Atomic metadata and one result per Trial. An exclusive lock fences resumes.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::workers::TrialResult;
use super::{Description, Task};

#[derive(Serialize, Deserialize)]
pub(super) struct Stored {
    format: u32,
    #[serde(flatten)]
    pub description: Description,
    pub state: State,
    created_at: String,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub verdict: Option<super::grading::Verdict>,
    #[serde(skip)]
    pub directory: PathBuf,
    #[serde(skip)]
    lock: Option<File>,
}

#[derive(Serialize)]
struct View<'a> {
    #[serde(flatten)]
    description: &'a Description,
    state: State,
    created_at: &'a str,
    results: Vec<TrialResult>,
    verdict: &'a Option<super::grading::Verdict>,
    minutes: std::collections::BTreeMap<&'static str, f64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum State {
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
        description: Description,
        tasks: impl FnOnce() -> Result<Vec<Task>>,
    ) -> Result<Self> {
        fs::create_dir_all(root)?;
        let directory = root.join(&description.id);
        fs::create_dir_all(&directory)?;
        let lock = File::create(directory.join("lock"))?;
        lock.try_lock().context("Experiment is already running")?;
        let path = directory.join("experiment.json");
        let mut stored = if path.exists() {
            let mut stored: Self = serde_json::from_slice(&fs::read(path)?)?;
            ensure!(stored.format == 1, "unsupported Experiment format");
            if stored.description.bar != description.bar {
                ensure!(
                    !directory.join("started").exists(),
                    "Bar cannot change after the first Trial"
                );
                stored.description.bar = description.bar;
            }
            stored
        } else {
            Self {
                format: 1,
                description,
                state: State::Prepared,
                created_at: chrono::Local::now().to_rfc3339(),
                tasks: tasks()?,
                verdict: None,
                directory: PathBuf::new(),
                lock: None,
            }
        };
        stored.directory = directory;
        stored.lock = Some(lock);
        stored.save()?;
        Ok(stored)
    }

    pub fn save(&self) -> Result<()> {
        write(&self.directory.join("experiment.json"), self)
    }

    pub fn results(&self) -> Result<Vec<TrialResult>> {
        let mut results = Vec::new();
        for path in result_paths(&self.directory)? {
            results.push(serde_json::from_slice(&fs::read(path)?)?);
        }
        Ok(results)
    }

    fn view(&self) -> Result<View<'_>> {
        let results = self.results()?;
        let mut minutes = std::collections::BTreeMap::from([("baseline", 0.0), ("candidate", 0.0)]);
        for result in &results {
            for attempt in &result.attempts {
                *minutes.get_mut(result.arm.name()).unwrap() +=
                    attempt.timings.total_ms as f64 / 60_000.0;
            }
        }
        Ok(View {
            description: &self.description,
            state: self.state,
            created_at: &self.created_at,
            results,
            verdict: &self.verdict,
            minutes,
        })
    }

    pub fn render(&self, json: bool) -> Result<String> {
        let view = self.view()?;
        if json {
            Ok(serde_json::to_string(&view)?)
        } else {
            Ok(format!(
                "{} · {} · {} → {} · {} Trial results · Verdict: {} · Baseline: {:.3} minutes · Candidate: {:.3} minutes",
                self.description.id,
                self.state,
                self.description.baseline.commit,
                self.description.candidate.commit,
                view.results.len(),
                self.verdict
                    .as_ref()
                    .map_or("pending", |verdict| verdict.outcome()),
                view.minutes["baseline"],
                view.minutes["candidate"]
            ))
        }
    }
}

pub(super) fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("missing Experiment folder")?)?;
    serde_json::to_writer_pretty(&mut temporary, value)?;
    temporary.flush()?;
    temporary.persist(path)?;
    Ok(())
}

fn result_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let root = directory.join("results");
    if !root.exists() {
        return Ok(paths);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|ext| ext == "json")
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

pub(super) fn show(root: &Path, id: Option<&str>, json: bool) -> Result<String> {
    if let Some(id) = id {
        ensure!(
            id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid Experiment ID"
        );
        let directory = root.join(id);
        let mut stored: Stored = serde_json::from_slice(
            &fs::read(directory.join("experiment.json")).context("Experiment not found")?,
        )?;
        stored.directory = directory;
        return stored.render(json);
    }
    let mut experiments = Vec::new();
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let path = entry?.path().join("experiment.json");
            if path.is_file() {
                let mut stored: Stored = serde_json::from_slice(&fs::read(&path)?)?;
                stored.directory = path.parent().unwrap().into();
                experiments.push(stored);
            }
        }
    }
    experiments.sort_by(|a, b| a.description.id.cmp(&b.description.id));
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
