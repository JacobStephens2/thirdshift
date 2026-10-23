//! Completion facts, saved by their owner or reconstructed from older records.

use std::collections::HashSet;

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use super::command_record::{CommandRecord, StageName};
use super::usual_range::{UsualRange, UsualRanges};

#[derive(Serialize, Deserialize)]
pub(super) struct RunMetrics {
    pub(super) outcome: String,
    pub(super) repairs: usize,
    pub(super) ci_rounds: usize,
    pub(super) stages: Vec<StageMetrics>,
    pub(super) total_minutes: Option<f64>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct StageMetrics {
    pub(super) name: String,
    pub(super) seconds: Option<f64>,
    pub(super) minutes: Option<f64>,
    pub(super) usual_range: Option<UsualRange>,
    pub(super) position: Position,
    // Only the first stage referencing a Session holds its review counts.
    pub(super) review_findings: Option<ReviewFindings>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ReviewFindings {
    pub(super) standards: Option<u64>,
    pub(super) spec: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Position {
    Below,
    Within,
    Above,
    TooFewRuns,
}

impl RunMetrics {
    pub(super) fn of(
        record: &CommandRecord,
        ranges: &UsualRanges,
        outside: &mut impl super::effects::Outside,
    ) -> Self {
        Self::calculate(record, ranges, |log| {
            outside
                .review_summary(log)
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        })
    }

    /// Old records have no saved review counts or Usual ranges. Do not infer
    /// those from sidecars or today's history, or write anything back.
    pub(super) fn historical(record: &CommandRecord) -> Self {
        Self::calculate(record, &UsualRanges::default(), |_| None)
    }

    fn calculate(
        record: &CommandRecord,
        ranges: &UsualRanges,
        mut review: impl FnMut(&std::path::Path) -> Option<ReviewFindings>,
    ) -> Self {
        let mut sessions = HashSet::new();
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
                        review_findings: timing
                            .stage
                            .session_log
                            .as_deref()
                            .filter(|log| sessions.insert(log.to_path_buf()))
                            .and_then(&mut review),
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
