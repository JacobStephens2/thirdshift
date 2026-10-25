//! Retained attribution and a deterministic, seeded draw across Harness/Model
//! pairs. Hash ranks make the draw independent of GitHub's response ordering.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::harness::built_with;

#[derive(Serialize, Deserialize)]
pub(crate) struct PullRequest {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub merged_at: DateTime<Utc>,
    pub harness: String,
    pub harness_version: Option<String>,
    pub model: String,
    pub effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<super::grader::Assessment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_record: Option<std::path::PathBuf>,
    #[serde(default)]
    pub for_you: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<super::calibration::HumanScores>,
}

impl PullRequest {
    pub fn from_body(
        repository: String,
        number: u64,
        url: String,
        merged_at: DateTime<Utc>,
        body: &str,
    ) -> Option<Self> {
        let built = built_with::read(body)?;
        Some(Self {
            repository,
            number,
            url,
            merged_at,
            harness: built.harness.name().into(),
            harness_version: built.version,
            model: built.model,
            effort: built.effort,
            assessment: None,
            command_record: None,
            for_you: false,
            mine: None,
        })
    }
}

pub(super) fn draw(prs: Vec<PullRequest>, size: usize, seed: u64) -> Vec<PullRequest> {
    let rank = |value: &str| -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(seed.to_le_bytes());
        hash.update(value.as_bytes());
        hash.finalize().into()
    };
    let mut seen = BTreeSet::new();
    let mut pairs: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for pr in prs {
        if seen.insert((pr.repository.to_ascii_lowercase(), pr.number)) {
            pairs
                .entry((pr.harness.clone(), pr.model.clone()))
                .or_default()
                .push(pr);
        }
    }
    let mut pairs: Vec<_> = pairs.into_iter().collect();
    pairs.sort_by_key(|((harness, model), _)| rank(&format!("{harness}\0{model}")));
    for (_, group) in &mut pairs {
        group.sort_by_key(|pr| rank(&pr.url));
    }
    let mut sample = Vec::new();
    while sample.len() < size {
        let before = sample.len();
        for (_, group) in &mut pairs {
            if let Some(pr) = group.pop() {
                sample.push(pr);
            }
            if sample.len() == size {
                break;
            }
        }
        if before == sample.len() {
            break;
        }
    }
    sample
}
