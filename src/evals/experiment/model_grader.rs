//! One fresh Grader per Eval task. Only its blind packet enters the working
//! directory; the saved Arm mapping and validated response stay outside it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use rand::{SeedableRng, seq::SliceRandom};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Task, arms, store, trial_results};
use crate::harness::{Asked, Choice, ChosenBy, Harness, Settings};
use crate::session::private;
use crate::{interrupt, progress, prompt};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Score {
    pub label: String,
    pub passed: bool,
    pub reason: String,
    pub wrong_finding: bool,
    pub borderline: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    scores: Vec<Score>,
}

#[derive(Serialize, Deserialize)]
struct Grader {
    harness: arms::FixedHarness,
    model: Option<String>,
    effort: Option<String>,
}

impl Grader {
    fn choice(&self) -> Choice {
        Choice {
            harness: Harness::named(&self.harness.name).expect("validated Grader Harness"),
            model: self.model.clone(),
            effort: self.effort.clone(),
            chosen_by: ChosenBy::UserConfig,
        }
    }
}

/// Fix the Grader before Trials can change PATH or a later resume loads a
/// different User config. Reuse an Arm's pinned executable when appropriate.
pub(super) fn prepare(experiment: &store::Stored, settings: &Settings) -> Result<()> {
    let path = experiment.directory.join("grader.json");
    if path.exists() {
        return Ok(());
    }
    let description = &experiment.description;
    let default = Choice::of(&Asked::default(), settings);
    let choice = std::iter::once(default.harness)
        .chain(Harness::ALL)
        .find_map(|harness| {
            let configured = settings.of(harness);
            let model = configured.model.as_ref()?;
            let different = [&description.baseline, &description.candidate]
                .iter()
                .all(|arm| {
                    arm.model
                        .as_ref()
                        .is_none_or(|arm| !arm.eq_ignore_ascii_case(model))
                });
            (different && harness.installed()).then(|| Choice {
                harness,
                model: Some(model.clone()),
                effort: configured.effort.clone(),
                chosen_by: ChosenBy::UserConfig,
            })
        })
        .unwrap_or(default);
    let prepared = description.prepared.as_ref().context("Arms not prepared")?;
    let harness = [&prepared.baseline, &prepared.candidate]
        .into_iter()
        .find(|arm| arm.harness.name == choice.harness.name())
        .map(|arm| Ok(arm.harness.clone()))
        .unwrap_or_else(|| arms::fixed_harness(choice.harness))?;
    store::write(
        &path,
        &Grader {
            harness,
            model: choice.model,
            effort: choice.effort,
        },
    )
}

#[derive(Serialize)]
struct Packet {
    criteria: String,
    trials: Vec<Trial>,
}

#[derive(Serialize)]
struct Trial {
    label: String,
    outcome: crate::trial::Outcome,
    standards: Option<String>,
    spec: Option<String>,
}

pub(super) fn run(experiment: &store::Stored) -> Result<()> {
    let grader: Grader =
        serde_json::from_slice(&fs::read(experiment.directory.join("grader.json"))?)?;
    // Machine caps are read live, not fixed with the Bar.
    let caps = crate::config::UserConfig::load()?.trial_caps.with_env()?;
    let mut by_task: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for entry in trial_results::terminal(&experiment.directory)? {
        by_task
            .entry((entry.result().suite.clone(), entry.result().task.clone()))
            .or_default()
            .push(entry);
    }
    for task in &experiment.tasks {
        if task.manifest.session == crate::evals::format::Session::Process {
            continue;
        }
        interrupt::check()?;
        let key = Sha256::new()
            .chain_update(task.suite.as_bytes())
            .chain_update([0])
            .chain_update(task.name.as_bytes())
            .finalize();
        let directory = experiment
            .directory
            .join("graders")
            .join(format!("{key:x}"));
        fs::create_dir_all(&directory)?;
        let mut results = by_task
            .remove(&(task.suite.clone(), task.name.clone()))
            .unwrap_or_default();
        let expected = experiment.description.bar.trials_per_task[&task.suite] * 2;
        ensure!(results.len() == expected, "missing Trials for model Grader");
        let saved = directory.join("assessment.json");
        let scores: BTreeMap<trial_results::ScoreTarget, Score> = if saved.exists() {
            serde_json::from_slice(&fs::read(&saved)?)?
        } else {
            let seed = experiment.description.bar.seed ^ u64::from_le_bytes(key[..8].try_into()?);
            results.shuffle(&mut Pcg64::seed_from_u64(seed));
            let packet = packet(task, &results)?;
            progress::step("model Grader starting");
            let response = grade(
                &grader,
                &directory,
                &packet,
                experiment.description.bar.timeout_seconds,
                &caps,
            )?;
            let mut labels = BTreeSet::new();
            for score in &response.scores {
                ensure!(
                    packet.trials.iter().any(|trial| trial.label == score.label),
                    "unknown Grader label"
                );
                ensure!(labels.insert(&score.label), "duplicate Grader label");
                ensure!(
                    !score.reason.trim().is_empty(),
                    "Grader reason must not be empty"
                );
            }
            ensure!(labels.len() == packet.trials.len(), "missing Grader Scores");
            let scores: BTreeMap<_, _> = results
                .iter()
                .zip(&packet.trials)
                .map(|(entry, trial)| {
                    let score = response
                        .scores
                        .iter()
                        .find(|score| score.label == trial.label)
                        .unwrap();
                    Ok((entry.target()?, score.clone()))
                })
                .collect::<Result<_>>()?;
            // Publish only a complete, validated response. On resume it can
            // be applied again even if Stop interrupted the per-Trial writes.
            store::write(&saved, &scores)?;
            scores
        };
        trial_results::apply_scores(&mut results, &scores)?;
    }
    Ok(())
}

fn packet(task: &Task, results: &[trial_results::Terminal]) -> Result<Packet> {
    let mut trials = Vec::new();
    for entry in results {
        let result = entry.result();
        let output = result.attempts.last().context("Trial has no attempts")?;
        let reports = output.reports.read_and_blind()?;
        trials.push(Trial {
            label: format!("trial-{}", trials.len() + 1),
            outcome: result.outcome,
            standards: reports.standards,
            spec: reports.spec,
        });
    }
    Ok(Packet {
        criteria: task.manifest.criteria.clone(),
        trials,
    })
}

fn grade(
    grader: &Grader,
    directory: &Path,
    packet: &Packet,
    timeout: u64,
    caps: &crate::scope::Caps,
) -> Result<Response> {
    session(
        Session {
            kind: "model Grader",
            choice: &grader.choice(),
            binary: &grader.harness.path,
            directory,
            prompt: prompt::grader(),
            output: ("THIRDSHIFT_GRADER_SCORES", "scores.json", "Scores"),
            timeout: Duration::from_secs(timeout),
            caps,
        },
        packet,
    )
}

/// One fresh private Grader session over a blind packet.
pub(super) struct Session<'a> {
    pub kind: &'a str,
    pub choice: &'a Choice,
    pub binary: &'a Path,
    /// Each session gets a new `session-*` workspace here.
    pub directory: &'a Path,
    pub prompt: String,
    /// The variable naming the response file, its name, and what it holds.
    pub output: (&'a str, &'a str, &'a str),
    pub timeout: Duration,
    pub caps: &'a crate::scope::Caps,
}

/// Run the session on `packet` and decode the response it writes.
pub(super) fn session<T: serde::de::DeserializeOwned>(
    session: Session<'_>,
    packet: &impl Serialize,
) -> Result<T> {
    let Session {
        kind,
        choice,
        binary,
        directory,
        prompt,
        output: (variable, file, holds),
        timeout,
        caps,
    } = session;
    let workspace = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(directory)?
        .keep();
    let input = workspace.join("packet.json");
    let response = workspace.join(file);
    store::write(&input, packet)?;
    fs::write(workspace.join("prompt.md"), &prompt)?;
    private::run(private::Request {
        kind,
        choice,
        binary,
        workspace: &workspace,
        artifacts: &workspace,
        prompt: &prompt,
        timeout,
        caps,
        environment: &[("THIRDSHIFT_GRADER_PACKET", &input), (variable, &response)],
    })?;
    let bytes =
        private::read_session_file(&response, private::SESSION_OUTPUT_MAX).map_err(|error| {
            if private::is_missing_file(&error) {
                anyhow::anyhow!("{kind} left no {holds}")
            } else {
                error
            }
        })?;
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid {kind} response; see private Grader artifacts"))
}
