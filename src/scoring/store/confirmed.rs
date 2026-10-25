//! Narrow, read-only confirmed Bug origins, independent of full sample metadata.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{Store, Windowed};

/// One confirmed bug's PR and its optional authoritative local Ticket run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfirmedPullRequest {
    pub url: String,
    pub local_run: Option<(PathBuf, String)>,
}

pub(crate) fn read_confirmed_origins() -> Result<Vec<ConfirmedPullRequest>> {
    Store::open()?.confirmed_origins()
}

// Decode only the fields required by metrics. Full proposals require evidence,
// Scores and complete Built-with metadata that these retained reads never used.
#[derive(Deserialize)]
struct Origins {
    window_end: DateTime<Utc>,
    bug_origins: Option<Vec<Origin>>,
}

impl Windowed for Origins {
    fn window_end(&self) -> DateTime<Utc> {
        self.window_end
    }
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

impl Store {
    pub fn confirmed_origins(&self) -> Result<Vec<ConfirmedPullRequest>> {
        let mut latest = BTreeMap::new();
        for checkpoint in self.ordered::<Origins>()? {
            for origin in checkpoint.value.bug_origins.into_iter().flatten() {
                latest.insert(
                    origin.issue.trim_end_matches('/').to_ascii_lowercase(),
                    origin,
                );
            }
        }
        Ok(latest
            .into_values()
            .filter(|origin| origin.state == "confirmed")
            .filter_map(|origin| origin.settlement)
            .map(|settlement| {
                let pr = settlement.pull_request;
                ConfirmedPullRequest {
                    url: pr.url,
                    local_run: match pr.run {
                        Some(Attribution::Local {
                            command_record,
                            name,
                        }) => Some((command_record, name)),
                        _ => None,
                    },
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
