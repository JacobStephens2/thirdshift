//! One model Grader sees one packet. Attribution and validated Scores stay outside it.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::PullRequest;
use crate::config::scoring::Settings;
use crate::harness::Choice;
use crate::sandbox;
use crate::session::recording::{Input, Recording};
use crate::{logs, prompt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum Criterion {
    Correct,
    Scoped,
    Tested,
    Honest,
    Standards,
}

impl Criterion {
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
pub(super) struct Score {
    pub criterion: Criterion,
    pub passed: bool,
    pub reason: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Grader {
    pub harness: String,
    pub model: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Assessment {
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
    let output = tempfile::Builder::new()
        .prefix("grader-")
        .tempdir_in(packet.parent().context("packet has no parent")?)?;
    let sandbox = if sandbox::disabled() {
        None
    } else {
        sandbox::for_session(
            &choice,
            &sandbox::resolve_binary(choice.harness.name())?,
            packet,
            output.path(),
        )?
    };
    let prompt = prompt::scoring_grader();
    let adapter = choice.harness.adapter();
    let invocation = adapter.session(&choice, None, &prompt);
    let mut command = match &sandbox {
        Some(sandbox) => sandbox.command(&invocation.args)?,
        None => {
            let mut command = Command::new(adapter.name());
            command.args(&invocation.args);
            command
        }
    };
    command.current_dir(packet);
    let scores = packet.join("scores.json");
    if scores.try_exists()? {
        fs::remove_file(&scores)?;
    }
    let recording = Recording::capture();
    let repo = crate::issue::Repo {
        owner: "JacobStephens2".into(),
        name: "thirdshift".into(),
    };
    let kind = crate::session::Kind::ScoringGrader(ordinal);
    let log = recording.session_path(&logs::session_dir(&repo), "scoring", &kind.token())?;
    logs::session(crate::session::Logged::of(kind), &log);
    // Retained exports may consult the host's production history or launch
    // another CLI outside the sandbox. Scores need only the sandbox stream.
    let mut interpretation = adapter.interpretation(packet, &prompt).without_retained();
    if sandbox.is_some() {
        interpretation.set_home(&output.path().join("home"));
    }
    let ended = recording.run(
        &kind.token(),
        &choice,
        &mut command,
        Input {
            prompt: &prompt,
            stdin: invocation.stdin.as_deref(),
            stop_at_limit: true,
        },
        &log,
        interpretation,
    );
    let ended = ended.map_err(|error| match &sandbox {
        Some(sandbox) => sandbox
            .startup_failure(&error)
            .map(anyhow::Error::new)
            .unwrap_or(error),
        None => error,
    })?;
    ensure!(
        ended.killed.is_empty(),
        "Grader left unfinished background work"
    );
    // Never follow a packet-controlled symlink into another retained record.
    ensure!(
        fs::symlink_metadata(&scores).is_ok_and(|meta| meta.is_file()),
        "Grader left no regular scores.json file"
    );
    let response: Response =
        serde_json::from_slice(&fs::read(scores)?).context("invalid Grader Scores")?;
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
