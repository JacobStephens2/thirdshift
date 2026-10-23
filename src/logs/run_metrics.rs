//! Facts captured by a Run's own process at completion, never recomputed by readers.

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::command_record::{CommandRecord, StageName};
use super::usual_range::{UsualRange, UsualRanges};

#[derive(Serialize, Deserialize)]
pub(super) struct RunMetrics {
    outcome: String,
    repairs: usize,
    ci_rounds: usize,
    #[serde(default)]
    unaddressed_findings: Option<crate::pull_request::UnaddressedFindings>,
    stages: Vec<StageMetrics>,
    total_minutes: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct StageMetrics {
    name: String,
    seconds: Option<f64>,
    minutes: Option<f64>,
    usual_range: Option<UsualRange>,
    position: Position,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Position {
    Below,
    Within,
    Above,
    TooFewRuns,
}

impl RunMetrics {
    pub(super) fn of(
        record: &CommandRecord,
        ranges: &UsualRanges,
        unaddressed_findings: Option<crate::pull_request::UnaddressedFindings>,
    ) -> Self {
        let count = |name: StageName| {
            record
                .stages
                .iter()
                .filter(|stage| stage.name == name.name())
                .count()
        };
        Self {
            outcome: record
                .outcome
                .clone()
                .expect("a finished Run has an outcome"),
            repairs: count(StageName::Repair),
            ci_rounds: count(StageName::CiWatch),
            unaddressed_findings,
            stages: record
                .stage_times(None)
                .map(|timing| {
                    let seconds = timing.seconds();
                    let usual_range = ranges.get(&record.repository, &timing.stage.name);
                    let position = match seconds.zip(usual_range) {
                        Some((seconds, range)) if seconds < range.lower_seconds => Position::Below,
                        Some((seconds, range)) if seconds > range.upper_seconds => Position::Above,
                        Some(_) => Position::Within,
                        None => Position::TooFewRuns,
                    };
                    StageMetrics {
                        name: timing.stage.name.clone(),
                        seconds,
                        minutes: seconds.map(|seconds| seconds / 60.0),
                        usual_range,
                        position,
                    }
                })
                .collect(),
            total_minutes: record.ended_at.as_deref().and_then(|ended| {
                let began = DateTime::parse_from_rfc3339(&record.started_at).ok()?;
                let ended = DateTime::parse_from_rfc3339(ended).ok()?;
                let milliseconds = ended.signed_duration_since(began).num_milliseconds();
                (milliseconds >= 0).then_some(milliseconds as f64 / 60_000.0)
            }),
        }
    }
}
