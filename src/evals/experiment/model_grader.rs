//! One fresh Grader per Eval task. Only its blind packet enters the working
//! directory; the saved Arm mapping and validated response stay outside it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use rand::{SeedableRng, seq::SliceRandom};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Task, arms, store, trial_results};
use crate::harness::{Asked, Choice, ChosenBy, Harness, Settings};
use crate::session::recording::{Input, Recording};
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
    crate::resources::wait()?;
    let workspace = tempfile::Builder::new()
        .prefix("session-")
        .tempdir_in(directory)?
        .keep();
    let input = workspace.join("packet.json");
    let scores = workspace.join("scores.json");
    store::write(&input, packet)?;
    let prompt = prompt::grader();
    fs::write(workspace.join("prompt.md"), &prompt)?;
    let choice = grader.choice();
    let adapter = choice.harness.adapter();
    let invocation = adapter.trial_session(&choice, None, &prompt);
    let sandbox =
        crate::sandbox::for_session(&choice, &grader.harness.path, &workspace, &workspace)?;
    // The scope surrounds the sandbox, so the Harness and all its tools
    // share the Grader's cap and inherit its raised OOM score.
    let mut launched = match &sandbox {
        Some(prepared) => prepared
            .launch()
            .scoped_command(&invocation.args, &workspace, caps)?,
        None => {
            crate::scope::grader_command(&grader.harness.path, &invocation.args, &workspace, caps)?
        }
    };
    launched
        .command
        .stderr(fs::File::create(workspace.join("stderr"))?)
        .env("THIRDSHIFT_GRADER_PACKET", &input)
        .env("THIRDSHIFT_GRADER_SCORES", &scores);
    let scope = launched.scope;
    // The unit name stays in the workspace for `journalctl --user -u` after.
    fs::write(workspace.join("scope"), scope.unit())?;
    let ended = Recording::for_trial().run(
        "model Grader",
        &choice,
        &mut launched.command,
        Input {
            prompt: &prompt,
            stdin: invocation.stdin.as_deref(),
            deadline: Some(Instant::now() + Duration::from_secs(timeout)),
            private: true,
        },
        &workspace.join("stdout.jsonl"),
        adapter.interpretation(&workspace, &prompt),
    );
    interrupt::check()?;
    let ended = ended.map_err(|_| {
        if scope.oom_killed() {
            anyhow::anyhow!(
                "model Grader hit its memory cap ({}); see private Grader artifacts, \
                 and raise the trial.memory_max User config to retry",
                crate::scope::OOM_KILL_CAUSE
            )
        } else {
            anyhow::anyhow!("model Grader session failed; see private Grader artifacts")
        }
    })?;
    ensure!(
        ended.killed.is_empty(),
        "model Grader left unfinished background work"
    );
    serde_json::from_slice(&fs::read(scores).context("model Grader left no Scores")?)
        .map_err(|_| anyhow::anyhow!("invalid model Grader response; see private Grader artifacts"))
}
