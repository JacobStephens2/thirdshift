//! Discover Trial progress and retained sessions for both readers. File IDs are
//! selected from this inventory; a caller can never supply a filesystem path.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::store::{State, Stored};

#[derive(Serialize)]
pub(crate) struct RunningTrial {
    pub id: String,
    pub started_at: String,
}

#[derive(Serialize)]
pub(crate) struct SessionLog {
    pub id: String,
    pub trial_id: String,
    pub attempt: usize,
    pub trace: bool,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Started {
    #[serde(default)]
    index: usize,
    started_at: String,
}

pub(super) struct Active(PathBuf);

impl Active {
    pub fn start(trial: &Path, attempt: &Path, index: usize) -> Result<Self> {
        let started = Started {
            index,
            started_at: chrono::Local::now().to_rfc3339(),
        };
        super::store::write(&attempt.join("started.json"), &started)?;
        let active = trial.join("active.json");
        super::store::write(&active, &started)?;
        Ok(Self(active))
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn inventory(stored: &Stored) -> Result<(Vec<RunningTrial>, Vec<SessionLog>)> {
    let mut running = Vec::new();
    let trials = stored.directory.join("trials");
    if trials.is_dir() && stored.state == State::Running {
        for trial in fs::read_dir(&trials)? {
            let trial = trial?;
            let id = trial.file_name().to_string_lossy().into_owned();
            let active = trial.path().join("active.json");
            if !stored
                .directory
                .join("results")
                .join(format!("{id}.json"))
                .is_file()
            {
                let bytes = match fs::read(active) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                let started: Started = serde_json::from_slice(&bytes)?;
                running.push(RunningTrial {
                    id,
                    started_at: started.started_at,
                });
            }
        }
    }
    running.sort_by(|a, b| a.id.cmp(&b.id));
    let mut sessions = Vec::new();
    for attempt in super::trace::attempts(&stored.directory)? {
        let trial_id = attempt
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let started = attempt.join("started.json");
        let index = if started.is_file() {
            serde_json::from_slice::<Started>(&fs::read(started)?)?.index
        } else {
            // Older Arms retain the log path in each completed attempt.
            let pending = attempt.parent().unwrap().join("pending.json");
            if pending.is_file() {
                let result: super::TrialResult = serde_json::from_slice(&fs::read(pending)?)?;
                result
                    .attempts
                    .iter()
                    .position(|output| {
                        output
                            .log
                            .as_ref()
                            .is_some_and(|log| log.starts_with(&attempt))
                    })
                    .unwrap_or(0)
            } else {
                0
            }
        };
        let folder = attempt.join("session");
        if !folder.is_dir() {
            continue;
        }
        let confined = folder.canonicalize()?;
        if !confined.starts_with(stored.directory.canonicalize()?) {
            continue;
        }
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".jsonl") || name.ends_with(".spans.jsonl") {
                continue;
            }
            let selected = crate::logs::session::path(&path);
            if selected != path && selected.is_file() {
                continue;
            }
            let canonical = match path.canonicalize() {
                Ok(path) => path,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if !canonical.starts_with(&confined) {
                continue;
            }
            // A raw log and its later Session trace retain the same link.
            let original = crate::logs::session::original(&path);
            let id = format!(
                "{:x}",
                Sha256::digest(
                    original
                        .strip_prefix(&stored.directory)?
                        .as_os_str()
                        .as_encoded_bytes()
                )
            );
            sessions.push(SessionLog {
                id,
                trial_id: trial_id.clone(),
                attempt: index,
                trace: name.ends_with(".logs.jsonl"),
                path,
            });
        }
    }
    sessions.sort_by(|a, b| (&a.trial_id, a.attempt, &a.id).cmp(&(&b.trial_id, b.attempt, &b.id)));
    Ok((running, sessions))
}
