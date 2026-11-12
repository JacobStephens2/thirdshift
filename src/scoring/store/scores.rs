//! Narrow, read-only weekly Grader Scores for metrics, with each criterion's
//! calibration. Model Scores you are asked for stay withheld here too.

use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::super::calibration::{self, HumanScores};
use super::super::grader::{Criterion, Score};
use super::{Store, Windowed};

/// One retained Scoring week, by the start of its window.
pub(crate) struct ScoredWeek {
    pub start: DateTime<Utc>,
    pub pull_requests: Vec<ScoredPullRequest>,
}

/// One sampled pull request's model Scores. `scores` is empty while it is
/// ungraded, or while it is for you to score and yours are not saved yet.
pub(crate) struct ScoredPullRequest {
    pub repository: String,
    pub harness: String,
    pub model: String,
    pub scores: Vec<(Criterion, bool)>,
}

pub(crate) struct ScoreHistory {
    pub weeks: Vec<ScoredWeek>,
    pub calibrated: BTreeMap<Criterion, bool>,
}

pub(crate) fn read_scores() -> Result<ScoreHistory> {
    Store::open()?.scores()
}

// Decode only what pass rates and calibration need, so a checkpoint holding
// only Bug origins still reads.
#[derive(Deserialize)]
struct Sampled {
    window_start: Option<DateTime<Utc>>,
    window_end: DateTime<Utc>,
    #[serde(default)]
    sample: Vec<SampledPullRequest>,
}

impl Windowed for Sampled {
    fn window_end(&self) -> DateTime<Utc> {
        self.window_end
    }
}

#[derive(Deserialize)]
struct SampledPullRequest {
    url: String,
    repository: String,
    harness: String,
    model: String,
    #[serde(default)]
    for_you: bool,
    #[serde(default)]
    assessment: Option<Assessed>,
    #[serde(default)]
    mine: Option<HumanScores>,
}

#[derive(Deserialize)]
struct Assessed {
    scores: Vec<Score>,
}

impl Store {
    pub fn scores(&self) -> Result<ScoreHistory> {
        let weeks: Vec<_> = self
            .ordered::<Sampled>()?
            .into_iter()
            .map(|checkpoint| checkpoint.value)
            .collect();
        let calibration = calibration::measure_pairs(
            weeks.iter().flat_map(|week| &week.sample).filter_map(|pr| {
                Some((
                    pr.url.as_str(),
                    pr.mine.as_ref()?,
                    pr.assessment.as_ref()?.scores.as_slice(),
                ))
            }),
        );
        Ok(ScoreHistory {
            calibrated: calibration
                .into_iter()
                .map(|(criterion, state)| (criterion, state.calibrated))
                .collect(),
            weeks: weeks
                .into_iter()
                .filter_map(|week| {
                    Some(ScoredWeek {
                        start: week.window_start?,
                        pull_requests: week
                            .sample
                            .into_iter()
                            .map(|pr| ScoredPullRequest {
                                scores: match (&pr.assessment, pr.for_you && pr.mine.is_none()) {
                                    (Some(assessed), false) => assessed
                                        .scores
                                        .iter()
                                        .map(|score| (score.criterion, score.passed))
                                        .collect(),
                                    _ => Vec::new(),
                                },
                                repository: pr.repository,
                                harness: pr.harness,
                                model: pr.model,
                            })
                            .collect(),
                    })
                })
                .collect(),
        })
    }
}
