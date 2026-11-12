//! Human judgments are committed together; calibration uses only paired Scores.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Week;
use super::grader::{Criterion, Score};
use crate::terminal;

#[derive(Serialize, Deserialize)]
pub(crate) struct HumanScores {
    scored_at: DateTime<Utc>,
    pub scores: Vec<Score>,
}

impl HumanScores {
    pub(super) fn new(scores: Vec<Score>) -> Result<Self> {
        let criteria: BTreeSet<_> = scores.iter().map(|score| score.criterion).collect();
        ensure!(
            scores.len() == 5 && criteria.len() == 5,
            "all five Scores are required exactly once"
        );
        ensure!(
            scores.iter().all(|score| !score.reason.trim().is_empty()),
            "every Score needs a reason"
        );
        Ok(Self {
            scored_at: Utc::now(),
            scores,
        })
    }
}

pub(super) fn enter() -> Result<HumanScores> {
    let mut scores = Vec::new();
    for criterion in Criterion::ALL {
        let passed = loop {
            let answer = ask(&format!("{criterion:?} (pass/fail): "))?;
            match answer.trim().to_ascii_lowercase().as_str() {
                "pass" => break true,
                "fail" => break false,
                _ => eprintln!("Enter pass or fail."),
            }
        };
        let reason = loop {
            let reason = ask(&format!("{criterion:?} reason: "))?;
            if !reason.trim().is_empty() {
                break reason;
            }
            eprintln!("Every Score needs a reason.");
        };
        scores.push(Score {
            criterion,
            passed,
            reason,
        });
    }
    HumanScores::new(scores)
}

fn ask(prompt: &str) -> Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    terminal::line()?.context("all five Scores are required; your previous Scores are unchanged")
}

#[derive(Serialize)]
pub(crate) struct State {
    pub calibrated: bool,
    pub matches: usize,
    pub compared: usize,
}

pub(crate) type Calibration = BTreeMap<Criterion, State>;

pub(super) fn measure(weeks: &[Week]) -> Calibration {
    measure_pairs(weeks.iter().flat_map(|week| &week.sample).filter_map(|pr| {
        Some((
            pr.url.as_str(),
            pr.mine.as_ref()?,
            pr.assessment.as_ref()?.scores.as_slice(),
        ))
    }))
}

/// Each pair is a pull request's URL, your Scores and the model's Scores.
pub(super) fn measure_pairs<'a>(
    pairs: impl Iterator<Item = (&'a str, &'a HumanScores, &'a [Score])>,
) -> Calibration {
    let mut paired: Vec<_> = pairs.collect();
    paired.sort_by_key(|(url, mine, _)| (std::cmp::Reverse(mine.scored_at), *url));
    Criterion::ALL
        .into_iter()
        .map(|criterion| {
            let mut seen = BTreeSet::new();
            let agreements: Vec<_> = paired
                .iter()
                .filter_map(|(url, mine, model)| {
                    if !seen.insert(*url) {
                        return None;
                    }
                    let human = mine
                        .scores
                        .iter()
                        .find(|score| score.criterion == criterion)?;
                    let model = model.iter().find(|score| score.criterion == criterion)?;
                    Some(human.passed == model.passed)
                })
                .take(20)
                .collect();
            let matches = agreements.iter().filter(|&&agrees| agrees).count();
            (
                criterion,
                State {
                    calibrated: agreements.len() == 20 && matches >= 18,
                    matches,
                    compared: agreements.len(),
                },
            )
        })
        .collect()
}
