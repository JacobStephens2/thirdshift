//! Private Security finding records. Public repositories use draft advisories;
//! private repositories whose advisory endpoint is unavailable use issues.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::GitHub;
use crate::issue::IssueUrl;
use crate::labels::{Label, NEEDS_TRIAGE};
use crate::security::reproduction::{FixSize, Reproduction, Severity};

mod description;
use description::Description;

const SECURITY_FINDING: Label = Label::new(
    "security-finding",
    "A Security finding recorded privately for the Day shift",
);

#[derive(Clone, Copy)]
pub enum FixEnding {
    Failed,
    Succeeded,
}

enum FixUpdate {
    LinkPending,
    End(FixEnding),
}

enum RecordStorage {
    Advisories(Vec<Value>),
    Issues(Vec<Value>),
}

/// Loaded records retain lazy decoding: only a selected finding is validated.
pub struct SecurityRecords {
    storage: RecordStorage,
}

/// A resolved finding and whether this collection created its private record.
pub struct ResolvedFinding {
    pub record: SecurityRecord,
    pub created: bool,
}

impl SecurityRecords {
    pub(crate) fn advisories(records: Vec<Value>) -> Self {
        Self {
            storage: RecordStorage::Advisories(records),
        }
    }

    pub(crate) fn issues(records: Vec<Value>) -> Self {
        Self {
            storage: RecordStorage::Issues(records),
        }
    }

    /// Reuse every state without changing its grade or write-up. Remember a
    /// creation only after record and notification metadata validation succeeds.
    /// Remote effects of a failed creation are deliberately not rolled back.
    pub fn record_or_reuse(
        &mut self,
        draft: &DraftAdvisory,
        create: impl FnOnce(&Self, &DraftAdvisory) -> Result<SecurityRecord>,
    ) -> Result<ResolvedFinding> {
        let (records, field, _) = self.entries();
        if let Some(value) = records.iter().find(|record| {
            record[field]
                .as_str()
                .is_some_and(|text| Description(text).matches_fingerprint(&draft.fingerprint))
        }) {
            return Ok(ResolvedFinding {
                record: self.decode_created(value.clone())?,
                created: false,
            });
        }
        let record = create(self, draft)?;
        let record = self.decode_created(record.value)?;
        self.values_mut().push(record.value.clone());
        Ok(ResolvedFinding {
            record,
            created: true,
        })
    }

    /// Native-response decoding belongs to the record module and its adapters.
    pub(crate) fn decode_created(&self, value: Value) -> Result<SecurityRecord> {
        let data = self.decode_state(&value)?;
        let metadata = RecordedFinding::of_record(&value)?;
        Ok(SecurityRecord {
            data,
            metadata,
            value,
        })
    }

    /// The record layout belongs to its storage, not each rule that reads it.
    fn entries(&self) -> (&[Value], &'static str, &'static str) {
        match &self.storage {
            RecordStorage::Advisories(records) => (records, "description", "draft"),
            RecordStorage::Issues(records) => (records, "body", "open"),
        }
    }

    fn values_mut(&mut self) -> &mut Vec<Value> {
        match &mut self.storage {
            RecordStorage::Advisories(records) | RecordStorage::Issues(records) => records,
        }
    }

    /// An unreproduced, untriaged finding still needs the Day shift's call.
    pub fn waiting_for_day_shift(&self, fixing: bool) -> bool {
        match &self.storage {
            RecordStorage::Advisories(records) => records.iter().any(|record| {
                record["state"] == "draft"
                    && (Description(record["description"].as_str().unwrap_or_default())
                        .reproduced_outcome()
                        .is_some()
                        && !fixing
                        && record["security_fix_closed"] != true
                        || Description(record["description"].as_str().unwrap_or_default())
                            .reproduced_outcome()
                            .is_none()
                            && record["severity"].is_null())
            }),
            RecordStorage::Issues(records) => records.iter().any(|record| {
                record["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("open"))
                    && (Description(record["body"].as_str().unwrap_or_default())
                        .reproduced_outcome()
                        .is_some()
                        && !fixing
                        && record["security_fix_closed"] != true
                        || Description(record["body"].as_str().unwrap_or_default())
                            .reproduced_outcome()
                            .is_none()
                            && record["labels"].as_array().is_some_and(|labels| {
                                labels.iter().any(|label| {
                                    label["name"]
                                        .as_str()
                                        .is_some_and(|name| NEEDS_TRIAGE.is_named(name))
                                })
                            }))
            }),
        }
    }

    /// A dispatched fix that failed and whose issue the Day shift has not closed.
    pub fn failed_fix(&self) -> Result<Option<IssueUrl>> {
        let (records, field, _) = self.entries();
        for record in records {
            if record["security_fix_closed"] == true {
                continue;
            }
            if let Some(issue) =
                Description(record[field].as_str().unwrap_or_default()).failed_fix()?
            {
                return Ok(Some(issue));
            }
        }
        Ok(None)
    }

    /// Most severe reproduced finding still awaiting a fix, ties in record order.
    /// A recorded Ticket has already been dispatched; its Run owns that fix.
    pub fn next_fix(&self) -> Result<Option<(SecurityRecord, RecordedFinding)>> {
        let (records, description, state) = self.entries();
        let mut next = None;
        for value in records {
            if !value["state"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(state))
                || Description(value[description].as_str().unwrap_or_default())
                    .fix_link()
                    .is_some()
            {
                continue;
            }
            if let Some((severity, _)) =
                Description(value[description].as_str().unwrap_or_default()).reproduced_outcome()
            {
                // The Day shift's current advisory grade takes precedence
                // over the historical proof-of-concept's score.
                let severity = value["severity"]
                    .as_str()
                    .and_then(Severity::parse)
                    .unwrap_or(severity);
                if next
                    .as_ref()
                    .is_none_or(|(most_severe, _)| severity < *most_severe)
                {
                    next = Some((severity, value));
                }
            }
        }
        next.map(|(severity, value)| {
            let mut metadata = RecordedFinding::of_record(value)?;
            metadata.severity = Some(severity.name().into());
            Ok((self.decode_created(value.clone())?, metadata))
        })
        .transpose()
    }

    fn decode_state(&self, value: &Value) -> Result<RecordData> {
        Ok(match &self.storage {
            RecordStorage::Advisories(_) => RecordData::Advisory {
                id: value["ghsa_id"]
                    .as_str()
                    .context("Security finding record has no advisory ID")?
                    .to_string(),
                description: value["description"]
                    .as_str()
                    .context("Security finding record has no description")?
                    .to_string(),
                untriaged: value["state"] == "draft" && value["severity"].is_null(),
            },
            RecordStorage::Issues(_) => RecordData::Issue {
                number: value["number"]
                    .as_u64()
                    .context("Security finding record has no issue number")?,
                description: value["body"]
                    .as_str()
                    .context("Security finding record has no body")?
                    .to_string(),
                untriaged: value["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("open"))
                    && value["labels"].as_array().is_some_and(|labels| {
                        labels.iter().any(|label| {
                            label["name"]
                                .as_str()
                                .is_some_and(|name| NEEDS_TRIAGE.is_named(name))
                        })
                    }),
            },
        })
    }
}

/// A finding read from the private storage selected for this repository.
enum RecordData {
    Advisory {
        id: String,
        description: String,
        untriaged: bool,
    },
    Issue {
        number: u64,
        description: String,
        untriaged: bool,
    },
}

/// Storage identity and native response fields remain private to this module.
pub struct SecurityRecord {
    data: RecordData,
    metadata: RecordedFinding,
    value: Value,
}

/// Only the metadata allowed in a Run notification; no private write-up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedFinding {
    pub severity: Option<String>,
    pub title: String,
    pub url: String,
}

impl RecordedFinding {
    fn of_record(record: &Value) -> Result<Self> {
        Ok(Self {
            severity: record["severity"].as_str().map(String::from),
            title: record["summary"]
                .as_str()
                .or_else(|| record["title"].as_str())
                .context("the Security finding's private record has no title")?
                .to_string(),
            url: record["html_url"]
                .as_str()
                .context("the Security finding's private record has no link")?
                .to_string(),
        })
    }
}

impl SecurityRecord {
    /// The existing literal-copy check; diagnostics never include private text.
    pub fn check_public_fix_text(&self, body: &str, private_url: &str) -> Result<()> {
        Description(self.description()).check_public_fix_text(body, private_url)
    }

    pub fn with_pending_fix(&self, current: &str, issue: &IssueUrl) -> Result<String> {
        Description(current).with_pending_fix(self.description(), issue)
    }

    pub fn with_fix_ending(
        &self,
        current: &str,
        issue: &IssueUrl,
        ending: FixEnding,
    ) -> Result<String> {
        Description(current).with_fix_ending(issue, ending)
    }

    pub fn with_reproduction(&self, reproduction: &Reproduction) -> String {
        Description(self.description()).with_reproduction(reproduction)
    }

    /// The pinned audit commit is validated only when reproduction needs it.
    pub fn audited_commit(&self) -> Result<&str> {
        Description(self.description()).audited_commit()
    }

    pub fn metadata(&self) -> &RecordedFinding {
        &self.metadata
    }

    pub fn private_issue_number(&self) -> Option<u64> {
        match &self.data {
            RecordData::Issue { number, .. } => Some(*number),
            RecordData::Advisory { .. } => None,
        }
    }

    /// The size judged by the completed reproduction, never the candidate write-up.
    pub fn fix_size(&self) -> Result<FixSize> {
        Description(self.description())
            .reproduced_outcome()
            .map(|(_, size)| size)
            .context("the Security finding has no reproduced fix size")
    }

    /// A Day-shift decision is the finding's grade; a repeated fingerprint
    /// must not publish new proof-of-concept evidence or replace that grade.
    pub fn untriaged(&self) -> bool {
        match &self.data {
            RecordData::Advisory { untriaged, .. } | RecordData::Issue { untriaged, .. } => {
                *untriaged
            }
        }
    }

    pub fn description(&self) -> &str {
        match &self.data {
            RecordData::Advisory { description, .. } | RecordData::Issue { description, .. } => {
                description
            }
        }
    }

    pub fn name(&self) -> String {
        match &self.data {
            RecordData::Advisory { id, .. } => id.clone(),
            RecordData::Issue { number, .. } => format!("issue #{number}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Package {
    pub ecosystem: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DraftAdvisory {
    pub fingerprint: String,
    pub summary: String,
    pub description: String,
    pub package: Package,
}

/// Validated finding facts and opaque evidence supplied by audit or review.
pub struct FindingDraft<'a> {
    pub fingerprint: &'a str,
    pub summary: &'a str,
    pub audited_commit: &'a str,
    pub provenance: FindingProvenance<'a>,
    pub original_description: &'a str,
    pub evidence: &'a str,
    pub package: Package,
}

#[derive(Clone, Copy)]
pub enum FindingProvenance<'a> {
    Audit,
    Review { issue_url: &'a str },
}

impl DraftAdvisory {
    pub fn new(input: FindingDraft<'_>) -> Self {
        Self {
            description: Description::draft(&input),
            fingerprint: input.fingerprint.into(),
            summary: input.summary.into(),
            package: input.package,
        }
    }
}

impl GitHub {
    /// Link the dispatched fix issue without changing the private record's grade.
    pub fn link_security_fix(
        &self,
        repo: &str,
        record: &SecurityRecord,
        issue: &IssueUrl,
    ) -> Result<()> {
        self.write_security_fix(repo, record, issue, FixUpdate::LinkPending)
    }

    /// Record the dispatch's ending, preserving any private edits made during it.
    pub fn record_security_fix_ending(
        &self,
        repo: &str,
        record: &SecurityRecord,
        issue: &IssueUrl,
        succeeded: bool,
    ) -> Result<()> {
        self.write_security_fix(
            repo,
            record,
            issue,
            FixUpdate::End(if succeeded {
                FixEnding::Succeeded
            } else {
                FixEnding::Failed
            }),
        )
    }

    fn write_security_fix(
        &self,
        repo: &str,
        record: &SecurityRecord,
        issue: &IssueUrl,
        update: FixUpdate,
    ) -> Result<()> {
        let (path, field) = match &record.data {
            RecordData::Advisory { id, .. } => (
                format!("repos/{repo}/security-advisories/{id}"),
                "description",
            ),
            RecordData::Issue { number, .. } => (format!("repos/{repo}/issues/{number}"), "body"),
        };
        let output = self.output_with_input(&["api", &path], None)?;
        if !output.status.success() {
            bail!(
                "reading the private record before linking its fix failed ({})",
                output.status
            );
        }
        let current: Value =
            serde_json::from_slice(&output.stdout).context("private record is invalid JSON")?;
        let current = current[field]
            .as_str()
            .context("the private record has no description")?;
        let description = match update {
            FixUpdate::LinkPending => record.with_pending_fix(current, issue)?,
            FixUpdate::End(ending) => record.with_fix_ending(current, issue, ending)?,
        };
        let body = serde_json::to_vec(&json!({field: description}))?;
        let output = self.output_with_input(
            &["api", "--method", "PATCH", &path, "--input", "-"],
            Some(&body),
        )?;
        if !output.status.success() {
            bail!(
                "linking the private record's fix failed ({})",
                output.status
            );
        }
        Ok(())
    }

    pub fn security_fix_text(&self, issue: &IssueUrl) -> Result<String> {
        let value = self.issue_view(issue, "title,body")?;
        let title = value["title"]
            .as_str()
            .context("the Security fix Ticket has no title")?;
        let body = value["body"]
            .as_str()
            .context("the Security fix Ticket has no body")?;
        Ok(format!("{title}\n\n{body}"))
    }
    /// Every state and every page. A 404 selects issues only when repository
    /// metadata confirms they will be private.
    pub fn security_records(&self, repo: &str) -> Result<SecurityRecords> {
        let records = match self.gh_api_items(
            &format!("repos/{repo}/security-advisories?per_page=100"),
            "",
        ) {
            Ok(records) => Ok(SecurityRecords::advisories(records)),
            Err(error) if format!("{error:#}").contains("(HTTP 404)") => {
                let repository = self.gh_json(&["api", &format!("repos/{repo}")])?;
                if repository["private"].as_bool() != Some(true) {
                    bail!(
                        "security advisories unavailable; refusing to record findings in public issues"
                    );
                }
                let records = self.gh_api_items(
                    &format!("repos/{repo}/issues?state=all&labels=security-finding&per_page=100"),
                    "",
                )?;
                Ok(SecurityRecords::issues(
                    records
                        .into_iter()
                        .filter(|record| record.get("pull_request").is_none())
                        .collect(),
                ))
            }
            Err(error) => Err(error),
        };
        let mut records = records?;
        let (_, field, _) = records.entries();
        for record in records.values_mut() {
            if let Some(link) = Description(record[field].as_str().unwrap_or_default()).fix_link() {
                let ticket = IssueUrl::parse(link)
                    .context("the private record has an invalid fix Ticket link")?;
                if !ticket.repo_slug().eq_ignore_ascii_case(repo) {
                    bail!("the private record's fix Ticket belongs to another repository");
                }
                record["security_fix_closed"] = json!(!self.issue_is_open(&ticket)?);
            }
        }
        Ok(records)
    }

    /// Use the storage selected while reading the repository's records.
    pub fn create_security_record(
        &self,
        repo: &str,
        records: &SecurityRecords,
        draft: &DraftAdvisory,
    ) -> Result<SecurityRecord> {
        let value = match &records.storage {
            RecordStorage::Advisories(_) => self.create_security_advisory(repo, draft),
            RecordStorage::Issues(_) => {
                self.ensure_labels(repo, &[SECURITY_FINDING, NEEDS_TRIAGE])?;
                let output = self.output_with_input(
                    &[
                        "issue",
                        "create",
                        "--repo",
                        repo,
                        "--title",
                        &draft.summary,
                        "--body-file",
                        "-",
                        "--label",
                        "security-finding,needs-triage",
                    ],
                    Some(draft.description.as_bytes()),
                )?;
                if !output.status.success() {
                    // API errors can echo private evidence, so do not relay them.
                    bail!(
                        "creating a private Security finding issue failed ({})",
                        output.status
                    );
                }
                let url = std::str::from_utf8(&output.stdout)
                    .context("creating a private Security finding issue returned invalid UTF-8")?;
                let issue = IssueUrl::parse(url.trim()).map_err(|_| {
                    anyhow::anyhow!(
                        "creating a private Security finding issue returned no issue URL"
                    )
                })?;
                let mut record = self.issue_view(&issue, "number,body,state,labels,title")?;
                record["html_url"] = json!(url.trim());
                Ok(record)
            }
        }?;
        records.decode_created(value)
    }

    /// Only the reproduction fields change; state, labels and disclosure stay
    /// with the Day shift. API diagnostics may contain private test evidence.
    pub fn update_security_record(
        &self,
        repo: &str,
        record: &SecurityRecord,
        reproduction: &Reproduction,
    ) -> Result<()> {
        if !record.untriaged() {
            bail!("refusing to replace a triaged Security finding record");
        }
        let (path, storage) = match &record.data {
            RecordData::Advisory { id, .. } => (
                format!("repos/{repo}/security-advisories/{id}"),
                SecurityRecords::advisories(Vec::new()),
            ),
            RecordData::Issue { number, .. } => (
                format!("repos/{repo}/issues/{number}"),
                SecurityRecords::issues(Vec::new()),
            ),
        };
        let observed = self.output_with_input(&["api", &path], None)?;
        if !observed.status.success() {
            bail!(
                "reading a private Security finding record before its update failed ({})",
                observed.status
            );
        }
        let observed: Value = serde_json::from_slice(&observed.stdout)
            .context("reading a private Security finding record returned invalid JSON")?;
        let current = storage.decode_state(&observed)?;
        let (description, untriaged) = match &current {
            RecordData::Advisory {
                description,
                untriaged,
                ..
            }
            | RecordData::Issue {
                description,
                untriaged,
                ..
            } => (description, *untriaged),
        };
        if !untriaged {
            bail!(
                "the Security finding was triaged during its reproduction; leaving the record unchanged"
            );
        }
        if description != record.description() {
            bail!(
                "the Security finding changed during its reproduction; leaving the record unchanged"
            );
        }
        if matches!(&record.data, RecordData::Advisory { .. })
            && matches!(reproduction.severity(), Some(Severity::Informational))
        {
            bail!(
                "informational severity has no GitHub advisory field; the Day shift must decide its representation; leaving the record unchanged"
            );
        }
        let description = record.with_reproduction(reproduction);
        let body = match &record.data {
            RecordData::Advisory { .. } => {
                json!({"description": description, "severity": reproduction.severity()})
            }
            RecordData::Issue { .. } => json!({"body": description}),
        };
        let body = serde_json::to_vec(&body)?;
        let output = self.output_with_input(
            &["api", "--method", "PATCH", &path, "--input", "-"],
            Some(&body),
        )?;
        if !output.status.success() {
            bail!(
                "updating a private Security finding record failed ({})",
                output.status
            );
        }
        Ok(())
    }

    /// The create endpoint creates a draft. No severity or version claim.
    fn create_security_advisory(&self, repo: &str, draft: &DraftAdvisory) -> Result<Value> {
        let body = serde_json::to_vec(&json!({
            "summary": draft.summary,
            "description": draft.description,
            "severity": null,
            "cwe_ids": [],
            "vulnerabilities": [{"package": draft.package, "vulnerable_version_range": null}]
        }))?;
        let output = self.output_with_input(
            &[
                "api",
                "--method",
                "POST",
                &format!("repos/{repo}/security-advisories"),
                "--input",
                "-",
            ],
            Some(&body),
        )?;
        if !output.status.success() {
            // API errors can echo private evidence, so do not relay them.
            bail!(
                "creating a draft repository security advisory failed ({})",
                output.status
            );
        }
        serde_json::from_slice(&output.stdout)
            .context("creating a draft advisory returned invalid JSON")
    }
}

#[cfg(test)]
#[path = "advisories_tests.rs"]
mod tests;
