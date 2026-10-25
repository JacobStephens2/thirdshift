//! Read the settled PR identities from the Scoring checkpoints (#1029).
//! Scoring owns the full schema; metrics need no packets, Scores or sessions.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::super::reading::{Command, Entry, Snapshot};

#[derive(Default)]
pub(super) struct ConfirmedCounts(BTreeMap<RunIdentity, usize>);

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct RunIdentity {
    record: PathBuf,
    name: String,
}

impl RunIdentity {
    fn of(command: &Command) -> Self {
        Self {
            record: command.record_path.clone(),
            name: command.record.name.clone(),
        }
    }
}

#[derive(Deserialize)]
struct Week {
    window_end: DateTime<Utc>,
    bug_origins: Option<Vec<Origin>>,
}

#[derive(Deserialize)]
struct Origin {
    issue: String,
    state: String,
    settlement: Option<Settlement>,
}

#[derive(Deserialize)]
struct Settlement {
    pull_request: PullRequest,
}

#[derive(Deserialize)]
struct PullRequest {
    url: String,
    run: Option<Attribution>,
}

#[derive(Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
enum Attribution {
    Local {
        command_record: PathBuf,
        name: String,
    },
    #[serde(other)]
    Other,
}

impl ConfirmedCounts {
    pub(super) fn read(snapshot: &Snapshot) -> Result<Self> {
        let directory = crate::config::home()?.join(".thirdshift/evals/scoring");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("could not read Scoring checkpoints"),
        };
        let mut weeks: Vec<Week> = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                weeks
                    .push(serde_json::from_slice(&fs::read(&path)?).with_context(|| {
                        format!("can't read Scoring sample {}", path.display())
                    })?);
            }
        }
        weeks.sort_by_key(|week| week.window_end);
        let mut latest = BTreeMap::new();
        for week in weeks {
            for origin in week.bug_origins.into_iter().flatten() {
                latest.insert(identity(&origin.issue), origin);
            }
        }
        let mut candidates = BTreeMap::new();
        for entry in snapshot.entries() {
            if let Entry::Record(command) = entry {
                collect(command, &mut candidates);
            }
        }
        let mut counts = Self::default();
        for origin in latest.values().filter(|origin| origin.state == "confirmed") {
            let Some(settlement) = &origin.settlement else {
                continue;
            };
            let pr = &settlement.pull_request;
            let Some(runs) = candidates.get(&identity(&pr.url)) else {
                continue;
            };
            let run = match &pr.run {
                Some(Attribution::Local {
                    command_record,
                    name,
                }) => runs
                    .iter()
                    .copied()
                    .find(|run| &run.record_path == command_record && &run.record.name == name),
                // Match Scoring's PR-to-Run lookup: the latest retained Run.
                // A Continuation can share its PR with earlier Runs.
                _ => runs.iter().copied().max_by_key(|run| {
                    (
                        DateTime::parse_from_rfc3339(&run.record.started_at).ok(),
                        &run.record_path,
                        &run.record.name,
                    )
                }),
            };
            if let Some(run) = run {
                *counts.0.entry(RunIdentity::of(run)).or_default() += 1;
            }
        }
        Ok(counts)
    }

    pub(super) fn for_run(&self, command: &Command) -> usize {
        self.0.get(&RunIdentity::of(command)).copied().unwrap_or(0)
    }
}

fn collect<'a>(command: &'a Command, candidates: &mut BTreeMap<String, Vec<&'a Command>>) {
    if let Some(url) = &command.record.pull_request {
        candidates.entry(identity(url)).or_default().push(command);
    }
    for child in command.dispatched.iter().chain(&command.children) {
        collect(child, candidates);
    }
}

fn identity(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}
