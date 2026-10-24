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

use super::{Task, arms, store, workers::TrialResult};
use crate::harness::{Asked, Choice, ChosenBy, Harness, Settings};
use crate::session::recording::{Input, Recording};
use crate::{interrupt, progress, prompt};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Score {
    label: String,
    pub passed: bool,
    reason: String,
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
        let mut results = Vec::new();
        for path in store::result_paths(&experiment.directory)? {
            let result: TrialResult = serde_json::from_slice(&fs::read(&path)?)?;
            if result.suite == task.suite && result.task == task.name {
                results.push((path, result));
            }
        }
        let expected = experiment.description.bar.trials_per_task[&task.suite] * 2;
        ensure!(results.len() == expected, "missing Trials for model Grader");
        let saved = directory.join("assessment.json");
        let scores: BTreeMap<String, Score> = if saved.exists() {
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
                .map(|((path, _), trial)| {
                    let score = response
                        .scores
                        .iter()
                        .find(|score| score.label == trial.label)
                        .unwrap();
                    Ok((
                        path.file_name()
                            .context("missing Trial filename")?
                            .to_str()
                            .context("invalid Trial filename")?
                            .to_owned(),
                        score.clone(),
                    ))
                })
                .collect::<Result<_>>()?;
            // Publish only a complete, validated response. On resume it can
            // be applied again even if Stop interrupted the per-Trial writes.
            store::write(&saved, &scores)?;
            scores
        };
        ensure!(
            scores.len() == results.len(),
            "saved Grader Scores do not cover Trials"
        );
        for (path, mut result) in results {
            let name = path.file_name().unwrap().to_str().unwrap();
            let score = scores.get(name).context("saved Grader Score missing")?;
            result
                .score
                .as_mut()
                .context("Trial has no mechanical Score")?
                .with_model(score.clone());
            store::write(&path, &result)?;
        }
    }
    Ok(())
}

fn packet(task: &Task, results: &[(std::path::PathBuf, TrialResult)]) -> Result<Packet> {
    let mut trials = Vec::new();
    for (_, result) in results {
        let output = result.attempts.last().context("Trial has no attempts")?;
        let report = |path: &Option<std::path::PathBuf>| -> Result<Option<String>> {
            path.as_ref()
                .map(|path| fs::read_to_string(path).map(|text| blind(&text)))
                .transpose()
                .map_err(Into::into)
        };
        trials.push(Trial {
            label: format!("trial-{}", trials.len() + 1),
            outcome: result.outcome,
            standards: report(&output.reports.standards)?,
            spec: report(&output.reports.spec)?,
        });
    }
    Ok(Packet {
        criteria: task.manifest.criteria.clone(),
        trials,
    })
}

fn blind(text: &str) -> String {
    enum Table {
        Identities,
        KeyValue,
        Findings,
    }

    let fields = ["harness", "model", "effort", "arm"];
    let mut table = None;
    let mut blinded = String::new();
    for line in text.lines() {
        let normalized = line.to_ascii_lowercase().replace(['*', '_', '`'], "");
        let normalized = normalized
            .trim()
            .trim_start_matches(['#', '>', '-', ' '])
            .trim();
        if normalized.contains('|') {
            let cells: Vec<_> = normalized
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            // Findings can name a Model or Harness as evidence. Metadata
            // tables instead identify Trials through columns or key/value rows.
            let kind = table.get_or_insert_with(|| {
                if cells.iter().any(|cell| fields.contains(cell)) {
                    Table::Identities
                } else if cells.len() == 2
                    && (cells[1] == "value"
                        || ["field", "key", "attribute", "property", "setting"].contains(&cells[0]))
                {
                    Table::KeyValue
                } else {
                    Table::Findings
                }
            });
            // A Findings block can still smuggle metadata: a two-column row
            // naming a field with a bare token value (| Harness | claude |)
            // identifies the Trial, while genuine finding evidence is a
            // sentence. Drop the former even when the block latched as
            // Findings.
            let metadata_row = cells.len() == 2
                && cells.first().is_some_and(|cell| fields.contains(cell))
                && cells[1].split_whitespace().count() <= 1;
            if matches!(kind, Table::Identities)
                || (matches!(kind, Table::KeyValue)
                    && cells.first().is_some_and(|cell| fields.contains(cell)))
                || metadata_row
            {
                continue;
            }
        } else {
            table = None;
        }
        let identifying = fields.iter().any(|key| {
            normalized == *key
                || normalized
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with([':', '=']))
        });
        if identifying
            || ["baseline", "candidate", "baseline arm", "candidate arm"].contains(&normalized)
            || ["built with", "generated with", "co-authored-by:"]
                .iter()
                .any(|prefix| normalized.starts_with(prefix))
        {
            continue;
        }
        blinded.push_str(line);
        blinded.push('\n');
    }
    blinded
}

fn grade(
    grader: &Grader,
    directory: &Path,
    packet: &Packet,
    timeout: u64,
    caps: &crate::scope::Caps,
) -> Result<Response> {
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
    let invocation = adapter.session(&choice, None, &prompt);
    // Each Grader runs in a scope of its own, with the Trial caps, and raises
    // its own `oom_score_adj`. A Grader the cap kills dies alone; its failure
    // below names the cap from the scope's `memory.events` evidence.
    let mut launched =
        crate::scope::grader_command(&grader.harness.path, &invocation.args, &workspace, caps)?;
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
