//! The guidance review's final-line contract. Only introduced findings may
//! appear here; pre-existing vulnerability details stay out of public text.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{fmt, fs};

use super::reproduction::{FixSize, Outcome as ReproductionOutcome, Reproduction, Severity};
use crate::github::{DraftAdvisory, FindingDraft, FindingProvenance, GitHub};
use crate::harness::interpretation::SafeguardRefusal;
use crate::issue::IssueUrl;
use crate::session::{Purpose, Sessions};
use crate::worktree::Worktree;
use crate::{logs, progress, prompt};

/// A fresh report outside the worktree, retained even when the session or
/// private recording fails. Only the introduced titles reach Delivery.
pub(crate) fn run(
    sessions: &Sessions<'_>,
    worktree: &Worktree,
    issue: &IssueUrl,
    base: &str,
    text: &str,
) -> Result<Review> {
    let merge_base = worktree.merge_base_commit(base)?;
    let root = logs::root(&issue.repo()).join("security-reviews");
    fs::create_dir_all(&root)?;
    let directory = tempfile::Builder::new()
        .prefix(&format!("{}-{}-", issue.number, logs::stamp()))
        .tempdir_in(root)?
        .keep();
    let report = directory.join("pre-existing.json");
    let text = text
        .replace(
            prompt::SECURITY_REVIEW_REPORT_FILE,
            &report.to_string_lossy(),
        )
        .replace(prompt::SECURITY_REVIEW_MERGE_BASE, &merge_base);
    progress::step(format!(
        "Security review private report: {}",
        report.display()
    ));
    let message =
        sessions.run_to_final_message(Purpose::SecurityReview, "security-review", &text)?;
    // A session may change commits on the Issue branch, but it cannot hand
    // us a replacement checkout to observe or publish findings from.
    worktree.head()?;
    let review = Review::from_final_message(message.as_deref())?;
    let findings: Vec<OldFinding> = serde_json::from_slice(&fs::read(report)?)?;
    if findings.len() != review.pre_existing_count {
        bail!("Security review's private report does not match its final count");
    }
    // Validate the entire report before creating any record. Malformed report
    // diagnostics are never passed into the public outcome.
    let findings = findings
        .into_iter()
        .map(|finding| {
            let reproduction = finding.reproduction()?;
            Ok((finding, reproduction))
        })
        .collect::<Result<Vec<_>>>()?;
    if !findings.is_empty() {
        let github = GitHub::new();
        let repo = issue.repo_slug();
        let mut known = github.security_records(&repo)?;
        let package = super::audit::package_in(worktree.path());
        for (finding, reproduction) in findings {
            let draft = DraftAdvisory::new(FindingDraft {
                fingerprint: &finding.fingerprint,
                summary: &finding.title,
                audited_commit: &merge_base,
                provenance: FindingProvenance::Review {
                    issue_url: &issue.url,
                },
                original_description: &finding.description,
                evidence: &serde_json::to_string_pretty(&finding)?,
                package: package.clone(),
            });
            // Reuse every record state and preserve the Day shift's grade.
            let resolved = known.record_or_reuse(&draft, |records, draft| {
                github.create_security_record(&repo, records, draft)
            })?;
            if resolved.created {
                github.update_security_record(&repo, &resolved.record, &reproduction)?;
                progress::step("recorded a pre-existing Security finding privately");
            }
        }
    }
    Ok(review)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OldFinding {
    fingerprint: String,
    title: String,
    description: String,
    proof_of_concept: ProofOfConcept,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProofOfConcept {
    test: String,
    command: String,
    head_exit_code: i32,
    merge_base_exit_code: i32,
    notes: String,
    severity: String,
    fix_size: String,
}

impl OldFinding {
    fn reproduction(&self) -> Result<Reproduction> {
        let proof = &self.proof_of_concept;
        if [
            &self.fingerprint,
            &self.title,
            &self.description,
            &proof.test,
            &proof.command,
            &proof.notes,
        ]
        .iter()
        .any(|text| text.trim().is_empty())
            || self.fingerprint.contains(['\r', '\n', '`'])
            || self.title.contains(['\r', '\n'])
            || proof.head_exit_code <= 0
            || proof.merge_base_exit_code <= 0
        {
            bail!(
                "Security review's private report lacks a valid old finding and failing proof-of-concept runs"
            );
        }
        let severity = Severity::parse(&proof.severity)
            .context("Security review's private report has an invalid reproduced severity")?;
        let size = match proof.fix_size.as_str() {
            "single" => FixSize::Single,
            "spec" => FixSize::Spec,
            _ => bail!("Security review's private report has an invalid reproduced fix size"),
        };
        Ok(Reproduction {
            outcome: ReproductionOutcome::Reproduced { severity, size },
            notes: format!(
                "Command: {}\nHEAD exit code: {}\nMerge base exit code: {}\n\n{}",
                proof.command, proof.head_exit_code, proof.merge_base_exit_code, proof.notes
            ),
            test: proof.test.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    Allow,
    Forbid,
}

pub(crate) enum Outcome {
    Complete(Review),
    Incomplete(String),
}

impl Outcome {
    pub fn of(result: Result<Review>) -> Self {
        match result {
            Ok(review) => Self::Complete(review),
            Err(error) => Self::Incomplete(match error.downcast_ref::<SafeguardRefusal>() {
                Some(refusal) => format!("Security review refused: {refusal}"),
                None => "Security review incomplete: session failed, ended early or omitted a valid final line; see Session log".to_string(),
            }),
        }
    }

    pub fn hold(&self) -> Option<Hold> {
        match self {
            Self::Complete(review) if review.findings.is_empty() => None,
            Self::Complete(review) => Some(Hold(format!(
                "Security review left {} unaddressed introduced finding(s): {}",
                review.findings.len(),
                review.findings.join("; ")
            ))),
            Self::Incomplete(reason) => Some(Hold(reason.clone())),
        }
    }

    pub fn entry(&self) -> String {
        match self {
            Self::Complete(review) if review.findings.is_empty() => {
                "No unaddressed introduced findings.".to_string()
            }
            Self::Complete(review) => review
                .findings
                .iter()
                .map(|title| format!("- {title}\n"))
                .collect(),
            Self::Incomplete(reason) => format!("- {reason}\n"),
        }
    }
}

/// Delivery has pushed and marked the PR ready before raising this hold.
#[derive(Debug)]
pub(crate) struct Hold(pub String);

impl fmt::Display for Hold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Hold {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    unaddressed_count: usize,
    pub findings: Vec<String>,
    pre_existing_count: usize,
}

impl Review {
    pub fn from_final_message(message: Option<&str>) -> Result<Self> {
        let json = message
            .and_then(|message| message.lines().rfind(|line| !line.trim().is_empty()))
            .and_then(|line| line.strip_prefix("Security review: "))
            .context("Security review ended without its required final line")?;
        let review: Self = serde_json::from_str(json)
            .context("Security review ended with an invalid final line")?;
        if review.unaddressed_count != review.findings.len()
            || review
                .findings
                .iter()
                .any(|title| title.trim().is_empty() || title.contains(['\r', '\n']))
        {
            bail!("Security review ended with an inconsistent finding count or title");
        }
        Ok(review)
    }
}
