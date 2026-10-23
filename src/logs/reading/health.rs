//! Health is an observation of live work, never a persisted Command fact.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Serialize;

use super::Command;
use crate::logs::command_record::{CommandRecord, CommandTree, StageName};

#[derive(Clone, Copy, Serialize)]
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

    fn get(&self, repository: &str, stage: &str) -> Option<UsualRange> {
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

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub(in crate::logs) enum Health {
    Ok,
    Silent {
        step: u8,
        seconds: u64,
        threshold_seconds: u64,
    },
    Slow {
        step: u8,
        usual_range: UsualRange,
        multiple: u64,
    },
}

impl Health {
    pub(in crate::logs) fn text(&self) -> Option<String> {
        match self {
            Self::Ok => None,
            Self::Slow {
                step,
                usual_range,
                multiple,
            } => {
                let mut text = format!("slower than usual ({}), still working", usual_range.text());
                if *step == 2 {
                    text.push_str(&format!(" · {multiple}× usual"));
                }
                Some(text)
            }
            Self::Silent {
                step,
                seconds,
                threshold_seconds,
            } => {
                let mut text = format!("silent {} min", seconds / 60);
                if *step == 2 {
                    text.push_str(&format!(
                        " · past {}",
                        threshold_seconds.saturating_mul(3) / 60
                    ));
                }
                Some(text)
            }
        }
    }
}

pub(super) fn observe(
    command: &mut Command,
    now: &DateTime<Local>,
    silent: Duration,
    outer_log: Option<&Path>,
    ranges: &UsualRanges,
) {
    let log = outer_log.unwrap_or(&command.command_log);
    let mut next_change = None;
    command.usual_ranges = command
        .record
        .stages
        .iter()
        .filter_map(|stage| {
            ranges
                .get(&command.record.repository, &stage.name)
                .map(|range| (stage.name.clone(), range))
        })
        .collect();
    if command.state == "live" {
        command.health = Some(Health::Ok);
        if let Some(stage) = &command.stage
            && !StageName::is_dispatch(&stage.name)
        {
            if let Some(range) = ranges.get(&command.record.repository, &stage.name) {
                // The first Slow step starts strictly above the upper range;
                // the second starts at twice that range, on sampled seconds.
                for threshold in [
                    range.upper_seconds.floor() as u64 + 1,
                    (range.upper_seconds * 2.0).ceil() as u64,
                ] {
                    schedule_transition(&mut next_change, now, threshold, stage.age_seconds as u64);
                }
                if stage.age_seconds as f64 > range.upper_seconds {
                    command.health = Some(Health::Slow {
                        step: if stage.age_seconds as f64 >= range.upper_seconds * 2.0 {
                            2
                        } else {
                            1
                        },
                        usual_range: range,
                        multiple: (stage.age_seconds as f64 / range.upper_seconds.max(1.0)).floor()
                            as u64,
                    });
                }
            }
            // A newly entered stage cannot already be Silent. Missing logs use
            // its start time, rather than inventing activity at read time.
            let mut quiet = stage.age_seconds as u64;
            let session = command
                .record
                .session_log
                .as_deref()
                .map(crate::logs::session::path);
            for path in std::iter::once(log).chain(session.as_deref()) {
                if let Ok(modified) = std::fs::metadata(path).and_then(|file| file.modified()) {
                    quiet = quiet.min(
                        now.signed_duration_since(DateTime::<Local>::from(modified))
                            .num_seconds()
                            .max(0) as u64,
                    );
                }
            }
            for threshold in [silent.as_secs(), silent.as_secs().saturating_mul(3)] {
                schedule_transition(&mut next_change, now, threshold, quiet);
            }
            if quiet >= silent.as_secs() {
                command.health = Some(Health::Silent {
                    step: if quiet >= silent.as_secs().saturating_mul(3) {
                        2
                    } else {
                        1
                    },
                    seconds: quiet,
                    threshold_seconds: silent.as_secs(),
                });
            }
        }
    }
    command.next_health_change = next_change;
    for child in command.children.iter_mut().chain(&mut command.dispatched) {
        observe(child, now, silent, Some(log), ranges);
    }
}

fn schedule_transition(
    change: &mut Option<DateTime<Local>>,
    now: &DateTime<Local>,
    threshold: u64,
    elapsed: u64,
) {
    let Some(remaining) = threshold
        .checked_sub(elapsed)
        .filter(|seconds| *seconds != 0)
    else {
        return;
    };
    let Some(at) = i64::try_from(remaining)
        .ok()
        .and_then(chrono::Duration::try_seconds)
        .and_then(|delay| now.checked_add_signed(delay))
    else {
        return;
    };
    *change = Some(change.map_or(at, |old| old.min(at)));
}
