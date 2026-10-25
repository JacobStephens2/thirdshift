//! Read the settled PR identities from the Scoring checkpoints (#1029).
//! Scoring owns the full schema; metrics need no packets, Scores or sessions.

use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Default)]
pub(super) struct ConfirmedCounts(BTreeMap<String, usize>);

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
}

impl ConfirmedCounts {
    pub(super) fn read() -> Result<Self> {
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
        let mut counts = Self::default();
        for origin in latest.values().filter(|origin| origin.state == "confirmed") {
            if let Some(settlement) = &origin.settlement {
                *counts
                    .0
                    .entry(identity(&settlement.pull_request.url))
                    .or_default() += 1;
            }
        }
        Ok(counts)
    }

    pub(super) fn for_pull_request(&self, url: Option<&str>) -> usize {
        url.and_then(|url| self.0.get(&identity(url)))
            .copied()
            .unwrap_or(0)
    }
}

fn identity(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}
