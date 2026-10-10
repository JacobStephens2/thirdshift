//! Private Security finding records. Public repositories use draft advisories;
//! private repositories whose advisory endpoint is unavailable use issues.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::GitHub;
use crate::issue::IssueUrl;
use crate::labels::{Label, NEEDS_TRIAGE};
use crate::security::reproduction::{FixSize, Reproduction, Severity};

const SECURITY_FINDING: Label = Label::new(
    "security-finding",
    "A Security finding recorded privately for the Day shift",
);

pub(crate) const FIX_TICKET_MARKER: &str = "\n<!-- thirdshift:security-fix -->\nFix Ticket: ";

const FIX_PENDING: &str = "Fix Run: pending";
const FIX_FAILED: &str = "Fix Run: failed";
const FIX_SUCCEEDED: &str = "Fix Run: succeeded";

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
        let marker = format!("Fingerprint: `{}`", draft.fingerprint);
        if let Some(value) = records.iter().find(|record| {
            record[field]
                .as_str()
                .is_some_and(|text| text.lines().any(|line| line == marker))
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
                    && (reproduced_severity(&record["description"]).is_some()
                        && !fixing
                        && record["security_fix_closed"] != true
                        || reproduced_severity(&record["description"]).is_none()
                            && record["severity"].is_null())
            }),
            RecordStorage::Issues(records) => records.iter().any(|record| {
                record["state"]
                    .as_str()
                    .is_some_and(|state| state.eq_ignore_ascii_case("open"))
                    && (reproduced_severity(&record["body"]).is_some()
                        && !fixing
                        && record["security_fix_closed"] != true
                        || reproduced_severity(&record["body"]).is_none()
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
            if let Some((_, fix)) = record[field]
                .as_str()
                .and_then(|text| text.rsplit_once(FIX_TICKET_MARKER))
                // The Pass lock rules out an active dispatch here. A pending
                // ending is also a failed/incomplete fix until success is recorded.
                && fix.lines().rev().find(|line| line.starts_with("Fix Run: "))
                    .is_some_and(|line| line == FIX_PENDING || line == FIX_FAILED)
            {
                return Ok(Some(IssueUrl::parse(
                    fix.lines().next().unwrap_or_default(),
                )?));
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
                || has_fix_ticket(&value[description])
            {
                continue;
            }
            if let Some((severity, _)) =
                reproduced_outcome(value[description].as_str().unwrap_or_default())
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

fn reproduced_severity(description: &Value) -> Option<Severity> {
    reproduced_outcome(description.as_str()?).map(|(severity, _)| severity)
}

fn reproduced_outcome(description: &str) -> Option<(Severity, FixSize)> {
    let (_, reproduction) =
        description.split_once("\n<!-- thirdshift:security-reproduction -->\n")?;
    let outcome = reproduction
        .lines()
        .find_map(|line| line.strip_prefix("Outcome: "))?;
    match outcome.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["reproduced", severity, "single"] => Some((Severity::parse(severity)?, FixSize::Single)),
        ["reproduced", severity, "spec"] => Some((Severity::parse(severity)?, FixSize::Spec)),
        _ => None,
    }
}

fn has_fix_ticket(description: &Value) -> bool {
    description
        .as_str()
        .is_some_and(|text| text.contains(FIX_TICKET_MARKER))
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
        reproduced_outcome(self.description())
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

impl GitHub {
    /// Link the dispatched fix issue without changing the private record's grade.
    pub fn link_security_fix(
        &self,
        repo: &str,
        record: &SecurityRecord,
        issue: &IssueUrl,
    ) -> Result<()> {
        self.write_security_fix(repo, record, issue, FIX_PENDING)
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
            if succeeded { FIX_SUCCEEDED } else { FIX_FAILED },
        )
    }

    fn write_security_fix(
        &self,
        repo: &str,
        record: &SecurityRecord,
        issue: &IssueUrl,
        ending: &str,
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
        let description = if ending == FIX_PENDING {
            if current != record.description() {
                bail!("the private record changed while publishing its fix; leaving it unchanged");
            }
            // Persist before dispatch: a later write failure cannot reopen Fencing.
            format!(
                "{}{FIX_TICKET_MARKER}{}\n{ending}\n",
                current.trim_end(),
                issue.url
            )
        } else {
            let link = current
                .rsplit_once(FIX_TICKET_MARKER)
                .and_then(|(_, fix)| fix.lines().next());
            if link != Some(issue.url.as_str()) {
                bail!(
                    "the private record's fix Ticket changed during its Run; leaving it unchanged"
                );
            }
            format!("{}\n{ending}\n", current.trim_end())
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
            if let Some((_, link)) = record[field]
                .as_str()
                .and_then(|text| text.rsplit_once(FIX_TICKET_MARKER))
            {
                let ticket = IssueUrl::parse(link.lines().next().unwrap_or_default())
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
        let description = reproduction.description(record.description());
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
