//! The Inbox's kept GitHub answer: what the last batched pull request read
//! showed, in one file per machine. `status` reuses it while it is under
//! five minutes old and reads GitHub first when it is missing or older; the
//! Console refreshes the same file once a minute. A row that has reached
//! the History page keeps what GitHub last showed and is not read again.
//! The reads never write to GitHub.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::reading::{Group, Snapshot};
use crate::config::UserConfig;
use crate::github::{GitHub, PullRequestState};

/// How long `status` reuses the kept answer before reading GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    pull_requests: BTreeMap<String, Option<PullRequestState>>,
    #[serde(default)]
    history: BTreeMap<String, Option<PullRequestState>>,
}

/// What the kept answer holds: every pull request read so far, and how old
/// the answer is. `None` age means no answer is kept. A pull request GitHub
/// can't resolve keeps `None`: what it last showed is nothing, and a row
/// that has reached History on that is not read again.
#[derive(Default)]
pub struct Kept {
    states: BTreeMap<String, Option<PullRequestState>>,
    history: BTreeMap<String, Option<PullRequestState>>,
    pub age: Option<Duration>,
}

impl Kept {
    pub fn state(&self, row: &str, url: &str) -> Option<&PullRequestState> {
        self.history
            .get(row)
            .or_else(|| self.states.get(url))
            .and_then(Option::as_ref)
    }

    pub fn has_answer(&self, url: &str) -> bool {
        self.states.contains_key(url)
    }

    pub fn is_frozen(&self, row: &str) -> bool {
        self.history.contains_key(row)
    }

    /// Preserve a History row's facts before another row refreshes the same
    /// pull request. The row selector distinguishes continuations and dispatches.
    pub fn freeze(&mut self, row: String, url: &str) {
        if let Some(state) = self.states.get(url) {
            self.history.entry(row).or_insert_with(|| state.clone());
        }
    }
}

/// The kept answer's file: one per machine, beside the logs rather than
/// among any Command's files, since it is a copy of GitHub, not something
/// a Command did.
pub fn path(logs_dir: &Path) -> PathBuf {
    logs_dir.join("github-inbox.json")
}

/// Freshness and temporary saves are bookkeeping, not changed Inbox rows.
pub fn bookkeeping(path: &Path, logs_dir: &Path) -> bool {
    path.parent() == Some(logs_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name == "github-inbox.refreshed" || name.starts_with(".github-inbox-")
            })
}

/// The kept answer, whatever its age. A missing or unreadable file keeps
/// nothing, so the next read starts over.
pub fn load(logs_dir: &Path) -> Kept {
    let path = path(logs_dir);
    let age = std::fs::metadata(path.with_extension("refreshed"))
        .or_else(|_| std::fs::metadata(&path))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    let file = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .filter(|file| file.version == 1);
    match file {
        Some(file) => Kept {
            states: file.pull_requests,
            history: file.history,
            age,
        },
        None => Kept::default(),
    }
}

/// Refresh without an age gate, even when no page is open. All this work,
/// including the blocking GitHub adapter, belongs off the Console's async thread.
pub fn poll() -> Result<()> {
    let config = UserConfig::load()?;
    let snapshot = Snapshot::load_with_silence(
        &config.logs_dir,
        chrono::Local::now(),
        config.silent_interval,
    )?;
    refresh(&snapshot, &config, Duration::ZERO)?;
    Ok(())
}

/// Read GitHub unless the kept answer is still fresh, and keep
/// the merged answer. A failed read keeps the last answer, however old.
/// Only reads; nothing here writes to GitHub.
pub(super) fn refresh(
    snapshot: &Snapshot,
    config: &UserConfig,
    fresh_for: Duration,
) -> Result<Kept> {
    let mut kept = load(&config.logs_dir);
    // From History on, preserve the row's last facts even if another row
    // still in view shares its pull request.
    for item in snapshot.inbox(true, config.inbox_days, &kept) {
        if item.group == Group::History
            && let Some(url) = item.pull_request()
            && let Some(row) = item.command.selector()
        {
            kept.freeze(row, url);
        }
    }
    let urls = snapshot.ready_pull_requests(config.inbox_days, &kept);
    if kept.age.is_some_and(|age| age < fresh_for) || urls.is_empty() {
        return Ok(kept);
    }
    match GitHub::new().pull_request_states(&urls) {
        Ok(states) => {
            for url in &urls {
                kept.states.insert(url.clone(), states.get(url).cloned());
            }
            kept.age = Some(Duration::ZERO);
            save(&config.logs_dir, &kept)?;
        }
        Err(_) => {
            // GitHub could not be read: the Inbox keeps the last answer.
        }
    }
    Ok(kept)
}

fn save(logs_dir: &Path, kept: &Kept) -> Result<()> {
    let path = path(logs_dir);
    let file = File {
        version: 1,
        pull_requests: kept.states.clone(),
        history: kept.history.clone(),
    };
    let bytes = serde_json::to_vec(&file)?;
    // Only changed facts replace the watched answer. A separate timestamp
    // keeps status from rereading GitHub after successful unchanged reads.
    if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
        let mut tmp = tempfile::Builder::new()
            .prefix(".github-inbox-")
            .tempfile_in(logs_dir)?;
        tmp.write_all(&bytes)?;
        tmp.persist(&path)?;
    }
    std::fs::write(path.with_extension("refreshed"), [])?;
    Ok(())
}
