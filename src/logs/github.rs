//! The Inbox's kept GitHub answer: what the last batched pull request read
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

use crate::github::{GitHub, PullRequestState};

/// How long `status` reuses the kept answer before reading GitHub again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    #[serde(default)]
    pull_requests: BTreeMap<String, Option<PullRequestState>>,
}

/// What the kept answer holds: every pull request read so far, and how old
/// the answer is. `None` age means no answer is kept. A pull request GitHub
/// can't resolve keeps `None`: what it last showed is nothing, and a row
/// that has reached History on that is not read again.
pub struct Kept {
    pub states: BTreeMap<String, Option<PullRequestState>>,
    pub age: Option<Duration>,
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
    let states = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<File>(&bytes).ok())
        .filter(|file| file.version == 1)
        .map(|file| file.pull_requests)
        .unwrap_or_default();
    Kept { states, age }
}

/// Read GitHub for `urls` unless the kept answer is still fresh, and keep
/// the merged answer. A failed read keeps the last answer, however old.
/// Only reads; nothing here writes to GitHub.
pub fn refresh(logs_dir: &Path, urls: &[String]) -> Result<Kept> {
    let mut kept = load(logs_dir);
    if kept.age.is_some_and(|age| age < FRESH_FOR) || urls.is_empty() {
        return Ok(kept);
    }
    match GitHub::new().pull_request_states(urls) {
        Ok(states) => {
            for url in urls {
                kept.states.insert(url.clone(), states.get(url).cloned());
            }
            kept.age = Some(Duration::ZERO);
            save(logs_dir, &kept.states)?;
        }
        Err(_) => {
            // GitHub could not be read: the Inbox keeps the last answer.
        }
    }
    Ok(kept)
}

fn save(logs_dir: &Path, states: &BTreeMap<String, Option<PullRequestState>>) -> Result<()> {
    let path = path(logs_dir);
    let file = File {
        version: 1,
        pull_requests: states.clone(),
    };
    let bytes = serde_json::to_vec(&file)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}
