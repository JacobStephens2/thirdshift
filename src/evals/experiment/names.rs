//! An Experiment's name: the Day shift's own, or one read from its Arms. It
//! stays outside the Bar and identity, so it never changes Trials or a resume.

use anyhow::{Result, ensure};

use super::Description;
use super::store::Details;

const LONGEST: usize = 100;

/// The trimmed name, or `None` when only whitespace was given.
pub(super) fn parse(text: &str) -> Result<Option<String>> {
    let name = text.trim();
    ensure!(
        !name.contains(['\n', '\r']),
        "Experiment name must be one line"
    );
    ensure!(
        name.chars().count() <= LONGEST,
        "Experiment name must be at most {LONGEST} characters"
    );
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// The one default for the CLI and Console: identical Arms first, then a
/// release, then what changed between the Arms.
pub(super) fn default(description: &Description, details: &Details) -> String {
    let (baseline, candidate) = (&description.baseline, &description.candidate);
    let at = || {
        details
            .candidate_ref
            .clone()
            .unwrap_or_else(|| short(&candidate.commit))
    };
    let changes: Vec<_> = [
        (
            "Harness",
            baseline.harness.as_str(),
            candidate.harness.as_str(),
        ),
        (
            "Model",
            baseline.model.as_deref().unwrap_or("default"),
            candidate.model.as_deref().unwrap_or("default"),
        ),
        (
            "Effort",
            baseline.effort.as_deref().unwrap_or("default"),
            candidate.effort.as_deref().unwrap_or("default"),
        ),
    ]
    .into_iter()
    .filter(|(_, b, c)| b != c)
    .map(|(label, b, c)| format!("{label}: {b} → {c}"))
    .collect();
    if baseline.commit == candidate.commit && changes.is_empty() {
        return format!("A/A at {}", at());
    }
    if details.release {
        return format!("Release {}", at());
    }
    if baseline.commit == candidate.commit {
        return changes.join(" · ");
    }
    match (&details.baseline_ref, &details.candidate_ref) {
        (Some(b), Some(c)) if b != c => format!("{b} → {c}"),
        (_, Some(c)) => c.clone(),
        _ => format!("{} → {}", short(&baseline.commit), short(&candidate.commit)),
    }
}

fn short(text: &str) -> String {
    text.chars().take(10).collect()
}
