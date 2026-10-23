//! Health is an observation of live work, never a persisted Command fact.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Local};
use serde::Serialize;

use super::Command;
use crate::logs::command_record::StageName;
use crate::logs::usual_range::{UsualRange, UsualRanges};

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
            if let Some(range) = ranges.get(&command.record.repository, &stage.name)
                && stage.age_seconds as f64 > range.upper_seconds
            {
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
    for child in command.children.iter_mut().chain(&mut command.dispatched) {
        observe(child, now, silent, Some(log), ranges);
    }
}
