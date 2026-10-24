//! The Inbox's kept GitHub answer: what the last batched issue/pull request read
//! showed, in one file per machine. `status` reuses it while it is under
//! five minutes old and reads GitHub first when it is missing or older; the
//! Console reads the same file. A row that has reached the History page
//! keeps what GitHub last showed and is not read again. The reads never
//! write to GitHub.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::github::{GitHub, InboxState, IssueState, PullRequestState};

/// How long `status` reuses the kept answer before reading GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default, alias = "pull_requests")]
    subjects: BTreeMap<String, Option<InboxState>>,
    #[serde(default)]
    history: BTreeMap<String, Option<InboxState>>,
}

/// What the kept answer holds: every issue/pull request read so far, and how old
/// the answer is. `None` age means no answer is kept. A subject GitHub
/// can't resolve keeps `None`: what it last showed is nothing, and a row
/// that has reached History on that is not read again.
#[derive(Default)]
pub struct Kept {
    states: BTreeMap<String, Option<InboxState>>,
    history: BTreeMap<String, Option<InboxState>>,
    pub age: Option<Duration>,
}

impl Kept {
    fn state(&self, row: &str, url: &str) -> Option<&InboxState> {
        self.history
            .get(&format!("{row}::{url}"))
            .or_else(|| self.history.get(row))
            .or_else(|| self.states.get(url))
            .and_then(Option::as_ref)
    }

    pub fn pull_request(&self, row: &str, url: &str) -> Option<&PullRequestState> {
        match self.state(row, url)? {
            InboxState::PullRequest(state) => Some(state),
            InboxState::Issue(_) => None,
        }
    }

    pub fn issue(&self, row: &str, url: &str) -> Option<&IssueState> {
        match self.state(row, url)? {
            InboxState::Issue(state) => Some(state),
            InboxState::PullRequest(_) => None,
        }
    }

    pub fn has_answer(&self, url: &str) -> bool {
        self.states.contains_key(url)
    }

    pub fn is_frozen(&self, row: &str, url: &str) -> bool {
        self.history.contains_key(row) || self.history.contains_key(&format!("{row}::{url}"))
    }

    /// Preserve a History row's facts before another row refreshes the same
    /// issue or pull request. The selector distinguishes continuations and dispatches.
    pub fn freeze(&mut self, row: String, url: &str) {
        if let Some(state) = self.states.get(url) {
            self.history
                .entry(format!("{row}::{url}"))
                .or_insert_with(|| state.clone());
        }
    }
}

/// The kept answer's file: one per machine, beside the logs rather than
/// among any Command's files, since it is a copy of GitHub, not something
/// a Command did.
pub fn path(logs_dir: &Path) -> PathBuf {
    logs_dir.join("github-inbox.json")
}

/// The kept answer, whatever its age. A missing or unreadable file keeps
/// nothing, so the next read starts over.
pub fn load(logs_dir: &Path) -> Kept {
    let path = path(logs_dir);
    let age = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    let file = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .filter(|file| file.version == 1);
    match file {
        Some(file) => Kept {
            states: file.subjects,
            history: file.history,
            age,
        },
        None => Kept::default(),
    }
}

/// Read GitHub for `urls` unless the kept answer is still fresh, and keep
/// the merged answer. A failed read keeps the last answer, however old.
/// Only reads; nothing here writes to GitHub.
pub fn refresh(logs_dir: &Path, urls: &[String], mut kept: Kept) -> Result<Kept> {
    if kept.age.is_some_and(|age| age < FRESH_FOR) || urls.is_empty() {
        return Ok(kept);
    }
    match GitHub::new().inbox_states(urls) {
        Ok(states) => {
            for url in urls {
                let mut state = states.get(url).cloned();
                if let Some(InboxState::Issue(issue)) = &mut state
                    && !issue.needs_triage
                {
                    issue.triaged_at = match kept.states.get(url) {
                        Some(Some(InboxState::Issue(previous))) if !previous.needs_triage => {
                            previous.triaged_at.clone()
                        }
                        _ => None,
                    }
                    .or_else(|| Some(chrono::Utc::now().to_rfc3339()));
                }
                kept.states.insert(url.clone(), state);
            }
            kept.age = Some(Duration::ZERO);
            save(logs_dir, &kept)?;
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
        subjects: kept.states.clone(),
        history: kept.history.clone(),
    };
    let bytes = serde_json::to_vec(&file)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
