//! Record a completed A/A Experiment's pooled, non-void Scores in its manifests.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::Serialize;

use super::store::{State, Stored};
use crate::evals::{format, suite};
use crate::trial::Outcome;

#[derive(Serialize)]
struct Reading {
    suite: String,
    task: String,
    difficulty: format::Difficulty,
    measured: bool,
    discriminating: bool,
}

pub(super) fn record(root: &Path, id: &str, json: bool) -> Result<String> {
    // Exclude running Trials and concurrent rechecks while capturing Scores.
    let experiment = Stored::load(root, id, true)?;
    ensure!(
        experiment.state == State::Completed,
        "Difficulty requires a completed Experiment"
    );
    let setup = &experiment.description.baseline;
    ensure!(
        setup == &experiment.description.candidate,
        "Difficulty requires A/A Arms with the same commit, Harness, Model and Effort"
    );
    let suites_root = crate::config::home()?.join(".thirdshift/evals/suites");
    let suites: BTreeSet<_> = experiment.tasks.iter().map(|task| &task.suite).collect();
    let mut locks = Vec::new();
    for name in suites {
        format::name(name)?;
        locks.push(suite::lock(&suites_root.join(name))?);
    }
    let results = experiment.results()?;
    let mut updates = Vec::new();
    // Prepare every document before writing, so bad or missing manifests do
    // not leave a partially recorded reading. Publication is atomic per file.
    for task in &experiment.tasks {
        format::name(&task.name)?;
        let directory = suites_root.join(&task.suite).join("tasks").join(&task.name);
        format::Task::load(&directory)?;
        let mut difficulty = format::Difficulty {
            passes: 0,
            trials: 0,
            experiment: id.into(),
            commit: setup.commit.clone(),
            harness: setup.harness.clone(),
            model: setup.model.clone(),
            effort: setup.effort.clone(),
        };
        for result in results.iter().filter(|result| {
            result.suite == task.suite
                && result.task == task.name
                && result.outcome != Outcome::Void
        }) {
            let score = result
                .score
                .as_ref()
                .context("non-void Trial has no Score")?;
            difficulty.trials += 1;
            difficulty.passes += usize::from(score.effective().passed);
        }
        let path = directory.join("task.toml");
        let mut document = std::fs::read_to_string(&path)?.parse::<toml_edit::DocumentMut>()?;
        let table = toml::to_string(&difficulty)?.parse::<toml_edit::DocumentMut>()?;
        document["difficulty"] = toml_edit::Item::Table(table.as_table().clone());
        let reading = Reading {
            suite: task.suite.clone(),
            task: task.name.clone(),
            measured: difficulty.measured(),
            discriminating: difficulty.discriminating(),
            difficulty,
        };
        updates.push((path, document, reading));
    }
    for (path, document, _) in &updates {
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().context("missing Eval task folder")?)?;
        file.write_all(document.to_string().as_bytes())?;
        file.flush()?;
        file.persist(path)?;
    }
    let readings: Vec<_> = updates.into_iter().map(|(_, _, reading)| reading).collect();
    if json {
        Ok(serde_json::to_string_pretty(&readings)?)
    } else {
        Ok(readings
            .iter()
            .map(|reading| {
                format!(
                    "{}/{}: Difficulty {}/{}{}; discriminating: {}",
                    reading.suite,
                    reading.task,
                    reading.difficulty.passes,
                    reading.difficulty.trials,
                    if reading.measured {
                        ""
                    } else {
                        " (unmeasured)"
                    },
                    reading.discriminating
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}
