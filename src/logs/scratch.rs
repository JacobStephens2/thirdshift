//! Scratch retention follows the owning Command's durable ending and identity.
use super::command_record::CommandTree;
use crate::scratch::{Retention, Work};
use anyhow::Result;
use std::path::Path;

pub(crate) fn read(path: &Path) -> Result<Retention> {
    let tree: CommandTree = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut keys = Vec::new();
    for facts in std::iter::once(&tree.command).chain(&tree.dispatched) {
        // Sharing a target is not enough: Triage and recommendation-only Review
        // do not continue Delivery. Other Commands use their exact command and
        // Launch directory, preserving their own pause/resume behavior.
        if matches!(
            facts.kind.as_str(),
            "Run" | "Spec run" | "Base fix" | "Fix run"
        ) {
            if let Some(issue) = &facts.issue {
                keys.push(Work::Issue(issue.url.clone()));
            }
            if let Some(pr) = &facts.pull_request {
                keys.push(Work::PullRequest(pr.clone()));
            }
        }
        if let Some(experiment) = &facts.experiment {
            keys.push(Work::Experiment(experiment.id.clone()));
        }
    }
    Ok(Retention {
        keep: tree.command.scratch_retained,
        completed: tree.command.outcome.as_deref().is_some_and(|outcome| {
            !matches!(outcome, "failed" | "paused" | "interrupted" | "skipped")
        }) && tree.command.cause.is_none(),
        keys,
    })
}
