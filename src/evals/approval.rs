//! The CLI and Console share one state-only, atomic approval.
use anyhow::Result;
use serde::Serialize;

use super::{format, suite};

pub(crate) struct Proposal {
    pub suite: String,
    pub task: String,
}

/// Proposals come from the same validated inventory as `evals list`.
pub(crate) fn proposed() -> Result<Vec<Proposal>> {
    let root = crate::config::home()?.join(".thirdshift/evals/suites");
    Ok(suite::list(&root)?
        .into_iter()
        .flat_map(|suite| {
            suite
                .tasks
                .into_iter()
                .filter(|task| task.state == format::State::Proposed)
                .map(move |task| Proposal {
                    suite: suite.name.clone(),
                    task: task.name,
                })
        })
        .collect())
}

#[derive(Serialize)]
pub(crate) struct Approval {
    suite: String,
    task: String,
    state: format::State,
}

pub(crate) fn approve(suite: &str, task: &str) -> Result<Approval> {
    format::name(suite)?;
    format::name(task)?;
    let directory = crate::config::home()?
        .join(".thirdshift/evals/suites")
        .join(suite);
    let _lock = suite::lock(&directory)?;
    let directory = directory.join("tasks").join(task);
    let manifest = format::Task::load(&directory)?;
    if manifest.state == format::State::Proposed {
        let path = directory.join("task.toml");
        let mut document = std::fs::read_to_string(&path)?.parse::<toml_edit::DocumentMut>()?;
        // Keep hand-written comments, formatting and every Trial input intact.
        let value = document["state"].as_value_mut().unwrap();
        let decor = value.decor().clone();
        *value = toml_edit::Value::from("approved");
        *value.decor_mut() = decor;
        format::write(&path, &document)?;
    }
    Ok(Approval {
        suite: suite.into(),
        task: task.into(),
        state: format::State::Approved,
    })
}
