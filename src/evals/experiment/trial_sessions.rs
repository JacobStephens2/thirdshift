//! Captured Trial/attempt layout and lexical Session file families. Discovery
//! never reads contents or applies a consumer's metadata or confinement policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::trial_results::files::{Files, directory};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Attempt {
    pub path: PathBuf,
    pub sessions: Vec<Session>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Session {
    pub original: PathBuf,
    pub raw: Option<PathBuf>,
    pub logs: Option<PathBuf>,
    pub spans: Option<PathBuf>,
}

impl Session {
    pub fn readable(&self) -> Option<&Path> {
        self.logs.as_deref().or(self.raw.as_deref())
    }
}

/// Whole export starts here, without acquiring result-storage dependencies.
pub(super) fn experiment(files: &impl Files, root: &Path) -> Result<Vec<Attempt>> {
    let mut attempts = Vec::new();
    for entry in directory(files, &root.join("trials"))? {
        attempts.extend(trial(files, &entry.path)?);
    }
    Ok(attempts)
}

/// Retained reading supplies its already captured Trial membership.
pub(super) fn trial(files: &impl Files, path: &Path) -> Result<Vec<Attempt>> {
    directory(files, path)?
        .into_iter()
        .filter(|entry| {
            entry.kind.is_dir()
                && entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("attempt-")
        })
        .map(|entry| attempt(files, &entry.path))
        .collect()
}

/// Finalization samples immediately before rewriting the Session traces.
pub(super) fn attempt(files: &impl Files, path: &Path) -> Result<Attempt> {
    let mut sessions = BTreeMap::new();
    for entry in directory(files, &path.join("session"))? {
        let name = entry.path.file_name().unwrap().to_string_lossy();
        let spans = name.ends_with(".spans.jsonl");
        // Span discovery intentionally has no readable-file type restriction.
        if !spans && !(entry.kind.is_file() && name.ends_with(".jsonl")) {
            continue;
        }
        let original = crate::logs::session::original(&entry.path);
        let session = sessions.entry(original.clone()).or_insert_with(|| Session {
            original,
            ..Default::default()
        });
        if spans {
            session.spans = Some(entry.path);
        } else if name.ends_with(".logs.jsonl") {
            session.logs = Some(entry.path);
        } else {
            session.raw = Some(entry.path);
        }
    }
    Ok(Attempt {
        path: path.into(),
        sessions: sessions.into_values().collect(),
    })
}

#[cfg(test)]
mod tests;
