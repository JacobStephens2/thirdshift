//! Repository stage-time ranges shared by live observations and completion metrics.

use std::collections::BTreeMap;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::command_record::{CommandRecord, CommandTree, StageName};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(in crate::logs) struct UsualRange {
    pub lower_seconds: f64,
    pub upper_seconds: f64,
}

impl UsualRange {
    pub(in crate::logs) fn text(self) -> String {
        format!(
            "{}–{} min",
            (self.lower_seconds / 60.0).round() as u64,
            (self.upper_seconds / 60.0).round() as u64
        )
    }
}

#[derive(Default)]
pub(super) struct UsualRanges(BTreeMap<(String, String), UsualRange>);

impl UsualRanges {
    pub(super) fn from_records(records: &BTreeMap<std::path::PathBuf, CommandTree>) -> Self {
        let mut repositories: BTreeMap<&str, Vec<&CommandTree>> = BTreeMap::new();
        for tree in records.values() {
            repositories
                .entry(&tree.command.repository)
                .or_default()
                .push(tree);
        }
        let mut samples: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
        for trees in repositories.values_mut() {
            trees.sort_by_key(|tree| {
                std::cmp::Reverse(DateTime::parse_from_rfc3339(&tree.command.started_at).ok())
            });
            for tree in trees.iter().take(30) {
                for record in std::iter::once(&tree.command).chain(&tree.dispatched) {
                    collect_times(record, &mut samples);
                }
            }
        }
        Self(
            samples
                .into_iter()
                .filter_map(|(key, mut times)| {
                    if times.len() < 3 {
                        return None;
                    }
                    times.sort_by(f64::total_cmp);
                    // Linear interpolation between adjacent ranks of the sorted sample.
                    let percentile = |fraction: f64| {
                        let rank = (times.len() - 1) as f64 * fraction;
                        let lower = rank.floor() as usize;
                        let upper = rank.ceil() as usize;
                        times[lower] + (times[upper] - times[lower]) * rank.fract()
                    };
                    Some((
                        key,
                        UsualRange {
                            lower_seconds: percentile(0.25),
                            upper_seconds: percentile(0.75),
                        },
                    ))
                })
                .collect(),
        )
    }

    pub(super) fn get(&self, repository: &str, stage: &str) -> Option<UsualRange> {
        self.0.get(&(repository.into(), stage.into())).copied()
    }
}

fn collect_times(record: &CommandRecord, samples: &mut BTreeMap<(String, String), Vec<f64>>) {
    for timing in record.stage_times(None) {
        let stage = timing.stage;
        if !StageName::known(&stage.name) || StageName::is_dispatch(&stage.name) {
            continue;
        }
        if let Some(seconds) = timing.seconds() {
            samples
                .entry((record.repository.clone(), stage.name.clone()))
                .or_default()
                .push(seconds);
        }
    }
}
