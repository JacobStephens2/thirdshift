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
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::reading::{Group, Snapshot};
use crate::config::UserConfig;
use crate::github::{GitHub, PullRequestState, Quotas};

/// How long `status` reuses the kept answer before reading GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    pull_requests: BTreeMap<String, Option<PullRequestState>>,
    #[serde(default)]
    history: BTreeMap<String, Option<PullRequestState>>,
    #[serde(default)]
    failure: Option<String>,
    #[serde(default)]
    quotas: Option<Quotas>,
    /// The warning at the last refresh, so crossing the age threshold
    /// wakes pages even when the facts and failure are unchanged.
    #[serde(default)]
    warning: Option<String>,
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
    last_answered: Option<DateTime<Utc>>,
    pub failure: Option<String>,
    quotas: Option<Quotas>,
}

impl Kept {
    pub fn last_answered_at(&self) -> Option<String> {
        self.last_answered
            .map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    pub fn warning(&self) -> Option<String> {
        let mut reasons = Vec::new();
        if let Some(reason) = self.quotas.as_ref().and_then(Quotas::warning) {
            reasons.push(reason);
        }
        if self.age.is_none_or(|age| age > FRESH_FOR)
            && let Some(failure) = &self.failure
        {
            reasons.push(format!("latest read failed: {failure}"));
        }
        if reasons.is_empty() {
            return None;
        }
        let reason = reasons.join("; ");
        Some(match self.last_answered_at() {
            Some(at) => format!("GitHub last answered at {at}; {reason}"),
            None => format!("GitHub has not answered yet; {reason}"),
        })
    }

    /// An open Inbox must also learn when a failed answer becomes stale.
    pub fn next_warning_change(&self) -> Option<Duration> {
        let age = self.age?;
        (age <= FRESH_FOR).then(|| FRESH_FOR - age + Duration::from_millis(1))
    }

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

/// Only github-inbox.changed wakes pages. The answer itself also keeps
/// History bookkeeping, which can change without changing an Inbox row.
pub fn bookkeeping(path: &Path, logs_dir: &Path) -> bool {
    path.parent() == Some(logs_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                matches!(name, "github-inbox.json" | "github-inbox.refreshed")
                    || name.starts_with(".github-inbox-")
            })
}

/// The kept answer, whatever its age. A missing or unreadable file keeps
/// nothing, so the next read starts over.
pub fn load(logs_dir: &Path) -> Kept {
    let path = path(logs_dir);
    let file = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .filter(|file| matches!(file.version, 1 | 2));
    match file {
        Some(file) => {
            let modified = std::fs::metadata(path.with_extension("refreshed"))
                // Version 1 used the answer's file time before it had a
                // separate freshness marker. Version 2 may hold only a failure.
                .or_else(|error| {
                    if file.version == 1 {
                        std::fs::metadata(&path)
                    } else {
                        Err(error)
                    }
                })
                .and_then(|metadata| metadata.modified())
                .ok();
            Kept {
                states: file.pull_requests,
                history: file.history,
                age: modified.and_then(|at| at.elapsed().ok()),
                last_answered: modified.map(DateTime::from),
                failure: file.failure,
                quotas: file.quotas,
            }
        }
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
    if snapshot
        .ready_pull_requests(config.inbox_days, &kept)
        .is_empty()
    {
        return Ok(kept);
    }
    // Serialize readers across status and Console processes. A busy reader
    // already owns the refresh; status can show the kept answer immediately.
    let lock = std::fs::File::create(config.logs_dir.join(".github-inbox-lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(kept),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    kept = load(&config.logs_dir);
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
    if kept.age.is_some_and(|age| age < fresh_for)
        || urls.is_empty()
        || kept.quotas.as_ref().and_then(Quotas::warning).is_some()
    {
        return Ok(kept);
    }
    let github = GitHub::new();
    match github.inbox_quotas() {
        Ok(quotas) => kept.quotas = Some(quotas),
        Err(error) => {
            kept.failure = Some(format!("{error:#}"));
            save(&config.logs_dir, &kept)?;
            return Ok(kept);
        }
    }
    for chunk in urls.chunks(100) {
        if kept.quotas.as_ref().and_then(Quotas::warning).is_some() {
            break;
        }
        match github.pull_request_states(chunk) {
            Ok((states, quota)) => {
                for url in chunk {
                    kept.states.insert(url.clone(), states.get(url).cloned());
                }
                kept.quotas
                    .as_mut()
                    .expect("quotas read before rows")
                    .graphql = quota;
                kept.age = Some(Duration::ZERO);
                kept.last_answered = Some(Utc::now());
                kept.failure = None;
            }
            Err(error) => {
                kept.failure = Some(format!("{error:#}"));
                break;
            }
        }
    }
    save(&config.logs_dir, &kept)?;
    Ok(kept)
}

fn save(logs_dir: &Path, kept: &Kept) -> Result<()> {
    let path = path(logs_dir);
    let file = File {
        version: 2,
        pull_requests: kept.states.clone(),
        history: kept.history.clone(),
        failure: kept.failure.clone(),
        quotas: kept.quotas.clone(),
        warning: kept.warning(),
    };
    let bytes = serde_json::to_vec(&file)?;
    let previous = std::fs::read(&path).ok();
    let facts_changed = previous
        .as_deref()
        .and_then(|bytes| serde_json::from_slice::<File>(bytes).ok())
        .is_none_or(|previous| {
            previous.pull_requests != file.pull_requests || previous.warning != file.warning
        });
    // Keep History freezes too, without waking pages for bookkeeping.
    if previous.as_deref() != Some(bytes.as_slice()) {
        let mut tmp = tempfile::Builder::new()
            .prefix(".github-inbox-")
            .tempfile_in(logs_dir)?;
        tmp.write_all(&bytes)?;
        tmp.persist(&path)?;
    }
    if let Some(at) = kept.last_answered {
        // Also migrate a version-1 answer that used its JSON file time.
        // A failed read must preserve that time, rather than erase it or
        // make the old facts look freshly read.
        let freshness = std::fs::File::create(path.with_extension("refreshed"))?;
        freshness.set_modified(at.into())?;
    }
    if facts_changed {
        std::fs::write(path.with_extension("changed"), [])?;
    }
    Ok(())
}
