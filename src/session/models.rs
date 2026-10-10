//! Per-session model evidence beside Session logs. Child Runs write their own
//! files so the owning command can include their models in its one notification.

use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::harness::Choice;
use crate::{logs, progress};

#[derive(Serialize, Deserialize)]
struct SessionModels {
    kind: String,
    harness: String,
    requested: Option<String>,
    effort: Option<String>,
    observed: Vec<String>,
}

/// Record only model metadata; failure costs attribution, never the session.
pub(super) fn keep(kind: &str, log: &Path, choice: &Choice, observed: Vec<String>) {
    let record = SessionModels {
        kind: kind.to_string(),
        harness: choice.harness.name().to_string(),
        requested: choice.model.clone(),
        effort: choice.effort.clone(),
        observed,
    };
    let path = log.with_extension("models.json");
    let kept = serde_json::to_vec(&record)
        .map_err(anyhow::Error::from)
        .and_then(|json| fs::write(&path, json).map_err(anyhow::Error::from));
    if let Err(error) = kept {
        progress::step(format_args!(
            "warning: could not record session models: {error:#}"
        ));
    }
}

/// Read this command's evidence, including child Runs sharing its start stamp.
/// Other commands in the same repository and folder are excluded.
pub fn notification_lines(command_log: &Path) -> Result<Vec<String>> {
    let Some(dir) = command_log.parent() else {
        return Ok(Vec::new());
    };
    let marker = format!("-{}-", logs::stamp());
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains(&marker) && name.ends_with(".models.json"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let record: SessionModels = serde_json::from_slice(&fs::read(path)?)?;
            let models = if record.observed.is_empty() {
                record.requested.map_or_else(
                    || "Harness default (not reported)".to_string(),
                    |model| format!("{model} (requested)"),
                )
            } else {
                let mut names = Vec::new();
                for model in record.observed {
                    if !names.contains(&model) {
                        names.push(model);
                    }
                }
                names.join(", ")
            };
            Ok(format!(
                "- {}: {} · {models} · session effort: {}",
                record.kind,
                record.harness,
                record.effort.as_deref().unwrap_or("default effort")
            ))
        })
        .collect()
}
