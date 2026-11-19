//! The guidance review's final-line contract. Only introduced findings may
//! appear here; pre-existing vulnerability details stay out of public text.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::{fmt, fs};

use super::reproduction::{FixSize, Outcome as ReproductionOutcome, Reproduction, Severity};
use crate::github::{DraftAdvisory, FindingDraft, FindingProvenance, GitHub};
use crate::harness::interpretation::{SafeguardRefusal, SessionFailure};
use crate::issue::{IssueUrl, Repo};
use crate::review::PullRequestUrl;
use crate::session::{Kind, Purpose, Sessions};
use crate::worktree::{SkillWorktree, Worktree};
use crate::{logs, progress, prompt};

pub(crate) mod step;

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
    run_in(sessions, issue, &merge_base, text, None, || {
        worktree.head()?;
        Ok(worktree.path())
    })
}

/// Keep private recording and the final-line contract shared with Ticket runs.
pub(crate) fn run_on_pull_request(
    sessions: &Sessions<'_>,
    worktree: &SkillWorktree,
    pr: &PullRequestUrl,
    base_commit: &str,
    text: &str,
) -> Result<Review> {
    let merge_base = worktree.merge_base_commit(base_commit)?;
    run_in(sessions, &pr.issue, &merge_base, text, None, || {
        worktree.head()?;
        Ok(worktree.path())
    })
}

/// Local review keeps validated private reports in the Trial, with no GitHub
/// recording. `base` is the Base commit sampled before sessions, so a later
/// move of the base ref cannot shrink the review diff.
pub(crate) fn run_local(
    sessions: &Sessions<'_>,
    repository: &Path,
    issue: &IssueUrl,
    base: &str,
    output: &Path,
    text: &str,
) -> Result<Review> {
    let git = crate::git::Git::new(repository);
    let merge_base = git.run(&["merge-base", base, "HEAD"])?;
    run_in(sessions, issue, &merge_base, text, Some(output), || {
        git.run(&["rev-parse", "HEAD"])?;
        Ok(repository)
    })
}

fn run_in<'a>(
    sessions: &Sessions<'_>,
    issue: &IssueUrl,
    merge_base: &str,
    text: &str,
    local_output: Option<&Path>,
    checkout: impl Fn() -> Result<&'a Path>,
) -> Result<Review> {
    let root = local_output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| logs::root(&issue.repo()))
        .join("security-reviews");
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
        .replace(prompt::SECURITY_REVIEW_MERGE_BASE, merge_base);
    progress::step(format!(
        "Security review private report: {}",
        report.display()
    ));
    let message =
        sessions.run_to_final_message(Purpose::SecurityReview, Kind::SecurityReview, &text)?;
    if local_output.is_some() {
        crate::trial::check_question(message.as_deref())?;
    }
    // A session may change commits on the Issue branch, but it cannot hand
    // us a replacement checkout to observe or publish findings from.
    let worktree = checkout()?;
    let review = Review::from_final_message(message.as_deref());
    // A deliberate incomplete still has a private report to validate and
    // record. Keep its public reason only if that processing succeeds.
    if let Err(error) = &review
        && !error.is::<Incomplete>()
    {
        return review;
    }
    // Validate the entire report before creating any record. Report errors
    // become a public-safe InvalidReport naming only the offending field and
    // finding index, never field values, so the real cause reaches the
    // outcome and Command log instead of the generic final-line message.
    let findings = match load_report(&report, &review) {
        Ok(findings) => findings,
        Err(error) => {
            if let Some(invalid) = error.downcast_ref::<InvalidReport>() {
                progress::step(format!("{invalid}"));
            }
            return Err(error);
        }
    };
    if local_output.is_none() && !findings.is_empty() {
        let github = GitHub::new();
        let repo = issue.repo_slug();
        let mut known = github.security_records(&repo)?;
        let package = super::audit::package_in(worktree);
        for (finding, reproduction) in findings {
            let draft = DraftAdvisory::new(FindingDraft {
                fingerprint: &finding.fingerprint,
                summary: &finding.title,
                audited_commit: merge_base,
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
    review
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
        for (field, text) in [
            ("fingerprint", &self.fingerprint),
            ("title", &self.title),
            ("description", &self.description),
            ("test", &proof.test),
            ("command", &proof.command),
            ("notes", &proof.notes),
        ] {
            if text.trim().is_empty() {
                bail!("has an empty `{field}` field");
            }
        }
        if self.fingerprint.contains(['\r', '\n', '`']) {
            bail!("has a `fingerprint` with a newline or backtick");
        }
        if self.title.contains(['\r', '\n']) {
            bail!("has a `title` with a newline");
        }
        if proof.head_exit_code <= 0 {
            bail!("has a non-positive `head_exit_code`");
        }
        if proof.merge_base_exit_code <= 0 {
            bail!("has a non-positive `merge_base_exit_code`");
        }
        let severity = Severity::parse(&proof.severity).context("has an invalid `severity`")?;
        let size = match proof.fix_size.as_str() {
            "single" => FixSize::Single,
            "spec" => FixSize::Spec,
            _ => bail!("has an invalid `fix_size`"),
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

/// A private report that fails to parse or validate. Its message is
/// public-safe: it names the report, the offending field and the 1-based
/// finding index, never any field value.
#[derive(Debug)]
struct InvalidReport(String);

impl fmt::Display for InvalidReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidReport {}

fn invalid_report(detail: impl fmt::Display) -> anyhow::Error {
    InvalidReport(format!(
        "Security review's private report is invalid: {detail}"
    ))
    .into()
}

const TOP_LEVEL_FIELDS: [&str; 4] = ["fingerprint", "title", "description", "proof_of_concept"];
const PROOF_OF_CONCEPT_FIELDS: [&str; 7] = [
    "test",
    "command",
    "head_exit_code",
    "merge_base_exit_code",
    "notes",
    "severity",
    "fix_size",
];

/// Map a field name reported by serde back to its static schema identifier
/// so untrusted JSON keys from the private report never enter `InvalidReport`.
fn known_schema_field(name: &str) -> Option<&'static str> {
    TOP_LEVEL_FIELDS
        .iter()
        .chain(PROOF_OF_CONCEPT_FIELDS.iter())
        .copied()
        .find(|&field| field == name)
}

/// The static schema field name in a serde unknown/missing-field error,
/// rejecting arbitrary JSON keys from the private report.
fn serde_field_name(error: &serde_json::Error) -> Option<&'static str> {
    let message = error.to_string();
    let raw = if let Some(rest) = message.strip_prefix("unknown field `") {
        rest.split_once("`, expected ")
            .or_else(|| rest.split_once("`, there are no fields"))
            .map(|(name, _)| name)
    } else if let Some(rest) = message.strip_prefix("missing field `") {
        rest.split_once('`').map(|(name, _)| name)
    } else {
        None
    }?;
    known_schema_field(raw)
}

/// Whether the finding object carries `field` at its top level, where a
/// nested missing-field error would otherwise misstate a misplaced field.
fn has_top_level_field(element: &[u8], field: &str) -> bool {
    serde_json::from_slice::<serde_json::Value>(element)
        .ok()
        .and_then(|value| value.as_object().map(|object| object.contains_key(field)))
        .unwrap_or(false)
}

/// The known schema field whose JSON type does not match `OldFinding` /
/// `ProofOfConcept`, ignoring arbitrary unknown keys. Names only, never values.
fn type_mismatch_field(element: &[u8]) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_slice(element).ok()?;
    let object = value.as_object()?;
    for field in TOP_LEVEL_FIELDS {
        let Some(value) = object.get(field) else {
            continue;
        };
        if field == "proof_of_concept" {
            let Some(proof) = value.as_object() else {
                return Some("proof_of_concept");
            };
            for proof_field in PROOF_OF_CONCEPT_FIELDS {
                let Some(proof_value) = proof.get(proof_field) else {
                    continue;
                };
                let mismatch = if matches!(proof_field, "head_exit_code" | "merge_base_exit_code") {
                    proof_value
                        .as_i64()
                        .is_none_or(|code| i32::try_from(code).is_err())
                } else {
                    !proof_value.is_string()
                };
                if mismatch {
                    return Some(proof_field);
                }
            }
        } else if !value.is_string() {
            return Some(field);
        }
    }
    None
}

/// Split a JSON array's top-level elements in document order, respecting
/// strings and nesting, so the first offending field in document order is
/// the one named. Returns `None` when the text is not a JSON array.
fn split_array_elements(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'[') || bytes.last() != Some(&b']') {
        return None;
    }
    let inner = &bytes[1..bytes.len() - 1];
    if inner.iter().all(|byte| byte.is_ascii_whitespace()) {
        return Some(Vec::new());
    }
    let mut elements = Vec::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut start = 0usize;
    for (offset, &byte) in inner.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                elements.push(&inner[start..offset]);
                start = offset + 1;
            }
            _ => {}
        }
    }
    if in_string || depth != 0 {
        return None;
    }
    elements.push(&inner[start..]);
    Some(elements)
}

/// Parse the private report, naming the first offending field and finding.
/// The struct definitions above remain the single schema source.
fn parse_report(bytes: &[u8]) -> Result<Vec<OldFinding>> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| invalid_report("invalid JSON"))?;
    if !value.is_array() {
        return Err(invalid_report("expected a JSON array of findings"));
    }
    let Some(elements) = split_array_elements(bytes) else {
        return Err(invalid_report("expected a JSON array of findings"));
    };
    let mut findings = Vec::with_capacity(elements.len());
    for (position, element) in elements.iter().enumerate() {
        let index = position + 1;
        if !element.iter().any(|byte| !byte.is_ascii_whitespace())
            || serde_json::from_slice::<serde_json::Value>(element)
                .map(|value| !value.is_object())
                .unwrap_or(true)
        {
            return Err(invalid_report(format!("finding {index} is not an object")));
        }
        let finding: OldFinding = serde_json::from_slice(element).map_err(|error| {
            let message = error.to_string();
            if message.starts_with("unknown field") {
                match serde_field_name(&error) {
                    Some(field) => {
                        invalid_report(format!("unknown field `{field}` at finding {index}"))
                    }
                    None => invalid_report(format!("unknown field at finding {index}")),
                }
            } else if let Some(field) = serde_field_name(&error) {
                // A field present at the top level but missing inside
                // proof_of_concept is misplaced, not missing.
                if has_top_level_field(element, field) {
                    invalid_report(format!("unknown field `{field}` at finding {index}"))
                } else {
                    invalid_report(format!("missing field `{field}` at finding {index}"))
                }
            } else if let Some(field) = type_mismatch_field(element) {
                invalid_report(format!(
                    "invalid type for field `{field}` at finding {index}"
                ))
            } else {
                invalid_report(format!("invalid field type at finding {index}"))
            }
        })?;
        findings.push(finding);
    }
    Ok(findings)
}

fn load_report(report: &Path, review: &Result<Review>) -> Result<Vec<(OldFinding, Reproduction)>> {
    let bytes = fs::read(report).map_err(|_| invalid_report("the report file cannot be read"))?;
    let findings = parse_report(&bytes)?;
    if let Ok(review) = review
        && findings.len() != review.pre_existing_count
    {
        return Err(invalid_report(format!(
            "it has {} entries but the final line reports {}",
            findings.len(),
            review.pre_existing_count
        )));
    }
    findings
        .into_iter()
        .enumerate()
        .map(|(position, finding)| {
            let reproduction = finding
                .reproduction()
                .map_err(|error| invalid_report(format!("finding {} {:#}", position + 1, error)))?;
            Ok((finding, reproduction))
        })
        .collect()
}

const MAX_REPORT_BYTES: u64 = 1024 * 1024;
const MIN_FRAGMENT_CHARS: usize = 8;

/// Identifying text from this repository's private Security reports.
/// Delivery captures it before a later session: that session can read the
/// report and then replace the file, and its final message is posted.
pub(crate) struct PrivateReportMaterial {
    fragments: Vec<String>,
}

impl PrivateReportMaterial {
    /// Whether `text` contains a stored report path or identifying field.
    /// Command strings are omitted; they are ordinary test invocations.
    pub(crate) fn exposes(&self, text: &str) -> bool {
        self.fragments
            .iter()
            .any(|fragment| text.contains(fragment))
    }
}

pub(crate) fn private_report_material(repo: &Repo) -> PrivateReportMaterial {
    private_report_material_under(&logs::root(repo))
}

/// [`private_report_material`], or none before the logs are configured.
pub(crate) fn private_report_material_if_configured(repo: &Repo) -> PrivateReportMaterial {
    logs::root_if_configured(repo).map_or_else(
        || PrivateReportMaterial {
            fragments: Vec::new(),
        },
        |root| private_report_material_under(&root),
    )
}

fn private_report_material_under(logs_root: &Path) -> PrivateReportMaterial {
    let mut fragments = Vec::new();
    let root = logs_root.join("security-reviews");
    if is_real_dir(&root)
        && let Ok(entries) = fs::read_dir(&root)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_real_dir(&path) {
                collect_report(&path.join("pre-existing.json"), &mut fragments);
            }
        }
    }
    PrivateReportMaterial { fragments }
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

fn collect_report(report: &Path, fragments: &mut Vec<String>) {
    let Some(bytes) = read_regular_report(report) else {
        return;
    };
    let path = report.to_string_lossy();
    push_fragment(fragments, &path);
    let Ok(findings) = serde_json::from_slice::<Vec<OldFinding>>(&bytes) else {
        return;
    };
    for finding in findings {
        let proof = finding.proof_of_concept;
        push_fragment(fragments, &finding.fingerprint);
        push_fragment(fragments, &finding.title);
        push_fragment(fragments, &finding.description);
        push_fragment(fragments, &proof.test);
        push_fragment(fragments, &proof.notes);
    }
}

fn push_fragment(fragments: &mut Vec<String>, text: &str) {
    let text = text.trim();
    // A validation report can quote one line without copying the whole field.
    for fragment in std::iter::once(text).chain(text.lines().filter(|line| *line != text)) {
        let fragment = fragment.trim();
        if fragment.chars().count() < MIN_FRAGMENT_CHARS {
            continue;
        }
        if let Ok(encoded) = serde_json::to_string(fragment)
            && let Some(escaped) = encoded
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
            && escaped != fragment
            && escaped.chars().count() >= MIN_FRAGMENT_CHARS
        {
            fragments.push(escaped.to_string());
        }
        fragments.push(fragment.to_string());
    }
}

/// A regular file no larger than [`MAX_REPORT_BYTES`], opened without
/// following a symlink. Anything else is skipped.
fn read_regular_report(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.file_type().is_file() || meta.len() > MAX_REPORT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_BYTES).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    Allow,
    Forbid,
}

/// How a Security review ended, with the Session trace of the session it ran,
/// which any Security hold it raises is reported with.
pub(crate) struct Outcome {
    ended: Ended,
    log: Option<PathBuf>,
}

enum Ended {
    Complete(Review),
    Incomplete(String),
    HarnessFailed {
        harness: &'static str,
        diagnostic: String,
    },
}

impl Outcome {
    /// Incomplete reviews count as an unaddressed finding, as in Delivery.
    pub fn unaddressed_count(&self) -> usize {
        match &self.ended {
            Ended::Complete(review) => review.findings.len(),
            Ended::Incomplete(_) | Ended::HarnessFailed { .. } => 1,
        }
    }

    /// The review that came to `result`, its session traceged to `log`, its
    /// Resume's if it got one, if it started one.
    pub fn of(result: Result<Review>, log: Option<PathBuf>) -> Self {
        let ended = match result {
            Ok(review) => Ended::Complete(review),
            Err(error) => {
                if let Some(refusal) = error.downcast_ref::<SafeguardRefusal>() {
                    Ended::Incomplete(format!("Security review refused: {refusal}"))
                } else if let Some(invalid) = error.downcast_ref::<InvalidReport>() {
                    Ended::Incomplete(format!("{invalid}"))
                } else if let Some(incomplete) = error.downcast_ref::<Incomplete>() {
                    Ended::Incomplete(format!("Security review incomplete: {incomplete}"))
                } else if let Some(failure) = error.downcast_ref::<SessionFailure>()
                    && let Some(diagnostic) = failure
                        .diagnostic
                        .as_deref()
                        .and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty()))
                {
                    Ended::HarnessFailed {
                        harness: failure.harness,
                        diagnostic: diagnostic.to_string(),
                    }
                } else {
                    Ended::Incomplete("Security review incomplete: session failed, ended early or omitted a valid final line; see Session trace".to_string())
                }
            }
        };
        Self { ended, log }
    }

    pub fn hold(&self) -> Option<Hold> {
        let reason = match &self.ended {
            Ended::Complete(review) if review.findings.is_empty() => return None,
            Ended::Complete(review) => format!(
                "Security review left {} unaddressed introduced finding(s): {}",
                review.findings.len(),
                review.findings.join("; ")
            ),
            Ended::Incomplete(reason) => reason.clone(),
            Ended::HarnessFailed {
                harness,
                diagnostic,
            } => format!("Security review on {harness} failed: {diagnostic}"),
        };
        Some(Hold {
            reason,
            log: self.log.clone(),
        })
    }

    pub fn entry(&self) -> String {
        match &self.ended {
            Ended::Complete(review) if review.findings.is_empty() => {
                "No unaddressed introduced findings.".to_string()
            }
            Ended::Complete(review) => review
                .findings
                .iter()
                .map(|title| format!("- {title}\n"))
                .collect(),
            Ended::Incomplete(reason) => format!("- {reason}\n"),
            // A failed result's diagnostic may be partial assistant output
            // with pre-existing vulnerability evidence. Keep it out of the PR.
            Ended::HarnessFailed { harness, .. } => {
                format!("- Security review on {harness} failed; see Session trace\n")
            }
        }
    }
}

/// Delivery has pushed and marked the PR ready before raising this hold.
/// It is reported with `log`, the Security review's Session trace, however
/// many sessions ran after the review.
#[derive(Debug)]
pub(crate) struct Hold {
    reason: String,
    log: Option<PathBuf>,
}

impl Hold {
    /// The Security review's Session trace, if it started a session.
    pub fn log(&self) -> Option<&Path> {
        self.log.as_deref()
    }
}

impl fmt::Display for Hold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Hold {}

/// A session's deliberate incomplete, distinct from invalid final lines and
/// private-report failures whose diagnostics must stay out of public text.
#[derive(Debug)]
struct Incomplete(String);

impl fmt::Display for Incomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Incomplete {}

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
        if let Some(reason) = json.strip_prefix("incomplete: ") {
            if reason.trim().is_empty() || reason.contains(['\r', '\n']) {
                bail!("Security review ended with an invalid incomplete reason");
            }
            return Err(Incomplete(reason.trim().chars().take(500).collect()).into());
        }
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

#[cfg(test)]
mod tests;
