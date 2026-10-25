//! Read the settled PR identities from the Scoring checkpoints (#1029).
//! Scoring owns the full schema; metrics need no packets, Scores or sessions.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::super::reading::{Command, CommandIdentity, PullRequestAttribution, Snapshot};

#[derive(Default)]
pub(super) struct ConfirmedCounts(BTreeMap<CommandIdentity, usize>);

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
                latest.insert(issue_identity(&origin.issue), origin);
            }
        }
        let attribution = PullRequestAttribution::new(snapshot);
        let mut counts = Self::default();
        for origin in latest.values().filter(|origin| origin.state == "confirmed") {
            let Some(settlement) = &origin.settlement else {
                continue;
            };
            let pr = &settlement.pull_request;
            let run = match &pr.run {
                Some(Attribution::Local {
                    command_record,
                    name,
                }) => attribution.exact(&pr.url, command_record, name),
                _ => attribution.latest(&pr.url),
            };
            if let Some(run) = run {
                *counts.0.entry(run.clone()).or_default() += 1;
            }
        }
        Ok(counts)
    }

    pub(super) fn for_run(&self, command: &Command) -> usize {
        self.0
            .get(&CommandIdentity::of(command))
            .copied()
            .unwrap_or(0)
    }
}

fn issue_identity(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}
