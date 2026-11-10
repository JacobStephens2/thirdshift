//! Private transcript checks and immutable human decisions about the real fix.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::session_evidence::Inventory;
use super::store::{State, Stored};
use super::trial_results::files::Native;
use super::{Task, TrialResult, trial_sessions};
use crate::trial::{Outcome, VoidCause};

pub(super) type History = BTreeMap<String, Decision>;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Hit {
    pub id: String,
    pub trial_id: String,
    pub attempt: usize,
    pub session: String,
    pub kind: String,
    pub identifier: String,
    pub line: usize,
    #[serde(default)]
    pub decision: Option<Decision>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Decision {
    pub decision: Choice,
    pub reason: String,
    pub reviewer: crate::logs::Actor,
    pub recorded_at: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Choice {
    Confirm,
    Dismiss,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Confirm => "confirm",
            Self::Dismiss => "dismiss",
        })
    }
}

impl Hit {
    pub fn status(&self) -> &'static str {
        match self.decision.as_ref().map(|decision| decision.decision) {
            None => "Pending decision",
            Some(Choice::Confirm) => "Confirmed leak",
            Some(Choice::Dismiss) => "Dismissed false positive",
        }
    }
}

fn identifiers(task: &Task) -> Result<Vec<(&'static str, String)>> {
    let manifest = &task.manifest;
    let mut values = vec![("merge_commit", manifest.change.clone())];
    let mut reference = |kind, url: &str| {
        if !url.is_empty() {
            values.push((kind, url.to_owned()));
            if let Some(number) = url.rsplit('/').next().filter(|s| s.parse::<u64>().is_ok()) {
                values.push((kind, number.into()));
            }
        }
    };
    if let Some(fix) = manifest
        .process_grading
        .as_ref()
        .and_then(|g| g.real_fix.as_ref())
    {
        reference("pull_request", &fix.pull_request);
        reference("issue", &fix.issue);
        values.extend([
            ("branch", fix.branch.clone()),
            ("title", fix.title.clone()),
            ("title", fix.issue_title.clone()),
        ]);
    } else {
        if let Some(inputs) = &manifest.trial {
            reference("pull_request", &inputs.pr_url);
            reference("issue", &inputs.issue_url);
            values.push(("branch", inputs.branch.clone()));
        }
        let source = task.directory.join(&manifest.repository);
        values.push((
            "title",
            super::super::repository::git(
                &source,
                &["show", "-s", "--format=%s", &manifest.change],
            )?
            .trim()
            .into(),
        ));
    }
    values.retain(|(_, value)| !value.trim().is_empty());
    values.sort();
    values.dedup();
    Ok(values)
}

fn mentions(text: &str, kind: &str, identifier: &str) -> bool {
    if kind == "merge_commit" {
        return text.split(|c: char| !c.is_ascii_hexdigit()).any(|token| {
            token.len() >= 7
                && identifier
                    .to_ascii_lowercase()
                    .starts_with(&token.to_ascii_lowercase())
        });
    }
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    text.match_indices(identifier).any(|(start, _)| {
        !text[..start].chars().next_back().is_some_and(word)
            && !text[start + identifier.len()..]
                .chars()
                .next()
                .is_some_and(word)
    })
}

// OTLP bodies contain JSON-encoded Harness events. Decode both layers so
// quoted titles and escaped tool output are checked as the reader sees them.
fn strings(value: &serde_json::Value, texts: &mut Vec<String>, depth: usize) {
    if depth > 64 {
        return;
    }
    match value {
        serde_json::Value::String(text) => {
            texts.push(text.clone());
            if let Ok(nested) = serde_json::from_str::<serde_json::Value>(text) {
                strings(&nested, texts, depth + 1);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                strings(value, texts, depth + 1);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                strings(value, texts, depth + 1);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

pub(super) fn scan(
    task: &Task,
    directory: &Path,
    attempt: &Path,
    trial: &str,
    index: usize,
) -> Result<Vec<Hit>> {
    if task.manifest.session != crate::evals::format::Session::Process {
        return Ok(Vec::new());
    }
    let identifiers = identifiers(task)?;
    let inventory = Inventory::new(&Native, directory)?;
    let attempt = trial_sessions::attempt(&Native, attempt)?;
    let mut hits = Vec::new();
    for selected in inventory.select(&Native, &attempt)? {
        let mut seen = BTreeSet::new();
        for (line, text) in BufReader::new(File::open(&selected.path)?)
            .lines()
            .enumerate()
        {
            let text = text?;
            let mut texts = vec![text.clone()];
            if let Ok(value) = serde_json::from_str(&text) {
                strings(&value, &mut texts, 0);
            }
            for (kind, identifier) in &identifiers {
                if seen.contains(&(*kind, identifier))
                    || !texts.iter().any(|text| mentions(text, kind, identifier))
                {
                    continue;
                }
                seen.insert((*kind, identifier));
                let id = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&(
                        trial,
                        index,
                        &selected.id,
                        kind,
                        identifier
                    ))?)
                );
                hits.push(Hit {
                    id,
                    trial_id: trial.into(),
                    attempt: index,
                    session: selected.id.clone(),
                    kind: (*kind).into(),
                    identifier: identifier.clone(),
                    line: line + 1,
                    decision: None,
                });
            }
        }
    }
    Ok(hits)
}

pub(super) fn apply(results: &mut [TrialResult], history: &History) {
    for result in results {
        for hit in &mut result.leaks {
            hit.decision = history.get(&hit.id).cloned();
            if hit
                .decision
                .as_ref()
                .is_some_and(|decision| decision.decision == Choice::Confirm)
            {
                result.outcome = Outcome::Void;
                if let Some(attempt) = result.attempts.get_mut(hit.attempt) {
                    attempt.outcome = Outcome::Void;
                    attempt.void_cause = Some(VoidCause::FixLeaked);
                    attempt.cause =
                        Some("Day shift confirmed the real fix leaked into the transcript".into());
                }
            }
        }
    }
}

pub(super) fn pending(results: &[TrialResult]) -> bool {
    results
        .iter()
        .flat_map(|result| &result.leaks)
        .any(|hit| hit.decision.is_none())
}

pub(super) fn confirmed(result: &TrialResult, history: &History) -> bool {
    result.leaks.iter().any(|hit| {
        history
            .get(&hit.id)
            .is_some_and(|decision| decision.decision == Choice::Confirm)
    })
}

pub(crate) fn decide(
    id: &str,
    hit: &str,
    choice: &str,
    reason: &str,
    reviewer: crate::logs::Actor,
    json: bool,
) -> Result<String> {
    let choice = match choice {
        "confirm" => Choice::Confirm,
        "dismiss" => Choice::Dismiss,
        _ => anyhow::bail!("leak decision must be confirm or dismiss"),
    };
    ensure!(!reason.trim().is_empty(), "a nonempty --reason is required");
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    let mut stored = Stored::load(&root, id, true)?;
    ensure!(
        stored.state != State::Running && stored.state != State::Prepared,
        "Experiment must stop before leak decisions"
    );
    let view = super::reading::read(&stored)?;
    let found = view
        .leak_hits
        .iter()
        .find(|found| found.id == hit)
        .context("leak hit not found")?;
    ensure!(found.decision.is_none(), "leak hit already decided");
    stored.leak_decisions.insert(
        hit.into(),
        Decision {
            decision: choice,
            reason: reason.trim().into(),
            reviewer,
            recorded_at: chrono::Local::now().to_rfc3339(),
        },
    );
    if stored.state == State::Completed {
        stored.refresh_verdict()?;
    }
    stored.save()?;
    super::retention::repositories(&stored);
    if let (Some(log), Some(verdict)) = (&stored.details.command_log, &stored.verdict)
        && let Err(error) = crate::logs::experiment_rechecked(log, id, verdict.outcome())
    {
        crate::progress::step(format!(
            "warning: could not update the Command record: {error:#}"
        ));
    }
    stored.render(json)
}
