//! Retained PR ownership and selection over an already captured Command tree.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};

use super::{Command, Entry, Snapshot};

/// Lexical storage identity: dispatched Commands can share a record path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::logs) struct CommandIdentity {
    pub record_path: PathBuf,
    pub name: String,
}

impl CommandIdentity {
    pub(in crate::logs) fn matches(&self, command: &Command) -> bool {
        self.record_path == command.record_path && self.name == command.record.name
    }

    pub(in crate::logs) fn of(command: &Command) -> Self {
        Self {
            record_path: command.record_path.clone(),
            name: command.record.name.clone(),
        }
    }
}

struct Candidate {
    identity: CommandIdentity,
    started_at: Option<DateTime<FixedOffset>>,
}

/// One matching and ordering rule for Scoring, Bug origins and Run metrics.
pub(in crate::logs) struct PullRequestAttribution {
    candidates: BTreeMap<String, Vec<Candidate>>,
}

impl PullRequestAttribution {
    pub(in crate::logs) fn new(snapshot: &Snapshot) -> Self {
        let mut attribution = Self {
            candidates: BTreeMap::new(),
        };
        for entry in snapshot.entries() {
            if let Entry::Record(command) = entry {
                attribution.collect(command);
            }
        }
        attribution
    }

    /// Invalid starts precede valid instants; lexical path/name break ties.
    pub(in crate::logs) fn latest(&self, url: &str) -> Option<&CommandIdentity> {
        self.candidates
            .get(&identity(url))?
            .iter()
            .max_by_key(|candidate| (candidate.started_at, &candidate.identity))
            .map(|candidate| &candidate.identity)
    }

    /// A saved local identity is authoritative, including its PR membership.
    pub(in crate::logs) fn exact(
        &self,
        url: &str,
        record_path: &Path,
        name: &str,
    ) -> Option<&CommandIdentity> {
        self.candidates
            .get(&identity(url))?
            .iter()
            .find(|candidate| {
                candidate.identity.record_path == record_path && candidate.identity.name == name
            })
            .map(|candidate| &candidate.identity)
    }

    fn collect(&mut self, command: &Command) {
        if let Some(url) = &command.record.pull_request {
            self.candidates
                .entry(identity(url))
                .or_default()
                .push(Candidate {
                    identity: CommandIdentity::of(command),
                    started_at: DateTime::parse_from_rfc3339(&command.record.started_at).ok(),
                });
        }
        // Snapshot has already excluded orphaned and unmatched child records.
        for child in command.children.iter().chain(&command.dispatched) {
            self.collect(child);
        }
    }
}

fn identity(url: &str) -> String {
    url.trim_end_matches('/').to_ascii_lowercase()
}
