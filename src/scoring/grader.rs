//! One model Grader sees one packet. Attribution and validated Scores stay outside it.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::PullRequest;
use crate::config::scoring::Settings;
use crate::harness::Choice;
use crate::prompt;
use crate::session::private;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Criterion {
    Correct,
    Scoped,
    Tested,
    Honest,
    Standards,
}

impl Criterion {
    pub fn name(self) -> &'static str {
        match self {
            Self::Correct => "Correct",
            Self::Scoped => "Scoped",
            Self::Tested => "Tested",
            Self::Honest => "Honest",
            Self::Standards => "Standards",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Correct,
        Self::Scoped,
        Self::Tested,
        Self::Honest,
        Self::Standards,
    ];
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Score {
    pub criterion: Criterion,
    pub passed: bool,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Grader {
    pub harness: String,
    pub model: String,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Assessment {
    pub grader: Grader,
    pub scores: Vec<Score>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    scores: Vec<Score>,
}

pub(super) fn run(
    settings: &Settings,
    pr: &PullRequest,
    packet: &Path,
    ordinal: usize,
) -> Result<Assessment> {
    let mut choice = settings.grader();
    if same_model(&choice, &pr.model) {
        choice = settings.fallback_grader();
    }
    ensure!(
        choice.model.is_some(),
        "configure an explicit Model in scoring.model or scoring.fallback_model so the Grader's independence can be checked"
    );
    ensure!(
        !same_model(&choice, &pr.model),
        "both Scoring Graders match the Run's Model; configure an independent scoring.model or fallback_model"
    );
    let prompt = prompt::scoring_grader();
    let scores = packet.join("scores.json");
    if scores.try_exists()? {
        fs::remove_file(&scores)?;
    }
    let repo = crate::issue::Repo {
        owner: "JacobStephens2".into(),
        name: "thirdshift".into(),
    };
    private::run_scoring(&choice, packet, &prompt, &repo, ordinal)?;
    // Never follow a packet-controlled symlink into another retained record.
    ensure!(
        fs::symlink_metadata(&scores).is_ok_and(|meta| meta.is_file()),
        "Grader left no regular scores.json file"
    );
    let response: Response = serde_json::from_slice(&private::read_session_file(
        &scores,
        private::SESSION_OUTPUT_MAX,
    )?)
    .context("invalid Grader Scores")?;
    let criteria: BTreeSet<_> = response
        .scores
        .iter()
        .map(|score| &score.criterion)
        .collect();
    ensure!(
        response.scores.len() == 5 && criteria.len() == 5,
        "Grader must supply all five criteria exactly once"
    );
    ensure!(
        response
            .scores
            .iter()
            .all(|score| !score.reason.trim().is_empty()),
        "every Score needs a reason"
    );
    Ok(Assessment {
        grader: Grader {
            harness: choice.harness.name().into(),
            model: choice.model.unwrap_or_else(|| "default model".into()),
        },
        scores: response.scores,
    })
}

fn same_model(choice: &Choice, model: &str) -> bool {
    choice
        .model
        .as_deref()
        .unwrap_or("default model")
        .trim()
        .eq_ignore_ascii_case(model.trim())
}
