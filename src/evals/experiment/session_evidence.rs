//! Trial Session evidence owns lexical identity, readable preference,
//! confinement and content access. Accounting never gates a selected read.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::store::Stored;
use super::trial_results::files::{Files, Native, optional};
use super::trial_sessions::{self, Attempt};

#[derive(Serialize)]
pub(crate) struct SessionLog {
    pub id: String,
    pub trial_id: String,
    pub attempt: usize,
    pub trace: bool,
}

pub(crate) struct Contents {
    pub id: String,
    pub trace: bool,
    pub contents: String,
}

pub(crate) fn read(id: &str, session: &str) -> Result<Contents> {
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    read_from(&Native, &root, id, session)
}

fn read_from(files: &impl Files, root: &Path, id: &str, session: &str) -> Result<Contents> {
    let stored = Stored::retained(files, root, id)?;
    let inventory = Inventory::new(files, &stored.directory)?;
    for attempt in trial_sessions::experiment(files, &stored.directory)? {
        if let Some(selected) = inventory
            .select(files, &attempt)?
            .into_iter()
            .find(|selected| selected.id == session)
        {
            // A selected file that disappears or becomes unreadable is
            // unavailable: never rescan or fall back to the raw log.
            let contents = String::from_utf8(files.read(&selected.path)?)?;
            return Ok(Contents {
                id: selected.id,
                trace: selected.trace,
                contents,
            });
        }
    }
    anyhow::bail!("Session not found")
}

struct Selected {
    id: String,
    trace: bool,
    path: PathBuf,
}

/// One canonical root for every evidence selection within an observation.
/// Full views supply their captured attempts; this module never rediscovers them.
pub(super) struct Inventory<'a> {
    directory: &'a Path,
    root: PathBuf,
}

impl<'a> Inventory<'a> {
    pub fn new(files: &impl Files, directory: &'a Path) -> Result<Self> {
        Ok(Self {
            directory,
            root: files.canonicalize(directory)?,
        })
    }

    pub fn listing(
        &self,
        files: &impl Files,
        attempt: &Attempt,
        trial_id: &str,
        index: usize,
    ) -> Result<Vec<SessionLog>> {
        Ok(self
            .select(files, attempt)?
            .into_iter()
            .map(|selected| SessionLog {
                id: selected.id,
                trial_id: trial_id.into(),
                attempt: index,
                trace: selected.trace,
            })
            .collect())
    }

    fn select(&self, files: &impl Files, attempt: &Attempt) -> Result<Vec<Selected>> {
        let mut readable = attempt
            .sessions
            .iter()
            .filter_map(|session| {
                session
                    .logs
                    .as_deref()
                    .or(session.raw.as_deref())
                    .map(|path| (session, path))
            })
            .peekable();
        let Some((_, first)) = readable.peek() else {
            return Ok(Vec::new());
        };
        let Some(confined) = optional(files.canonicalize(first.parent().unwrap()))? else {
            return Ok(Vec::new());
        };
        if !confined.starts_with(&self.root) {
            return Ok(Vec::new());
        }
        let mut selected = Vec::new();
        for (session, path) in readable {
            let Some(canonical) = optional(files.canonicalize(path))? else {
                continue;
            };
            if !canonical.starts_with(&confined) {
                continue;
            }
            // Original lexical bytes keep links stable when raw logs are
            // replaced by converted Session-trace log records.
            let id = format!(
                "{:x}",
                Sha256::digest(
                    session
                        .original
                        .strip_prefix(self.directory)
                        .context("Session is outside its Experiment")?
                        .as_os_str()
                        .as_encoded_bytes()
                )
            );
            selected.push(Selected {
                id,
                trace: session.logs.is_some(),
                path: path.into(),
            });
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests;
