//! Private Security finding records. Public repositories use draft advisories;
//! private repositories whose advisory endpoint is unavailable use issues.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::GitHub;
use crate::issue::IssueUrl;
use crate::labels::{Label, NEEDS_TRIAGE};
use crate::security::reproduction::{Reproduction, Severity};

const SECURITY_FINDING: Label = Label::new(
    "security-finding",
    "A Security finding recorded privately for the Day shift",
);

pub(crate) const FIX_TICKET_MARKER: &str = "\n<!-- thirdshift:security-fix -->\nFix Ticket: ";

pub enum SecurityRecords {
    Advisories(Vec<Value>),
    Issues(Vec<Value>),
}

impl SecurityRecords {
    /// An unreproduced, untriaged finding still needs the Day shift's call.
    pub fn waiting_for_day_shift(&self, fixing: bool) -> bool {
        match self {
            Self::Advisories(records) => records.iter().any(|record| {
                record["state"] == "draft"
                    && (reproduced_severity(&record["description"]).is_some()
                        && !fixing
                        && record["security_fix_closed"] != true
                        || reproduced_severity(&record["description"]).is_none()
                            && record["severity"].is_null())
            }),
            Self::Issues(records) => records.iter().any(|record| {
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

    /// Most severe reproduced finding still awaiting a fix, ties in record order.
    /// A recorded Ticket has already been dispatched; its Run owns that fix.
    pub fn next_fix(&self) -> Result<Option<(SecurityRecord, crate::security::RecordedFinding)>> {
        let (records, description, state) = match self {
            Self::Advisories(records) => (records, "description", "draft"),
            Self::Issues(records) => (records, "body", "open"),
        };
        let mut next = None;
        for value in records {
            if !value["state"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(state))
                || has_fix_ticket(&value[description])
            {
                continue;
            }
            if let Some(severity) = reproduced_severity(&value[description]) {
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
            let mut metadata = crate::security::RecordedFinding::of_record(value)?;
            metadata.severity = Some(severity.name().into());
            Ok((self.record(value)?, metadata))
        })
        .transpose()
    }

    pub fn remember(&mut self, record: Value) {
        match self {
            Self::Advisories(records) | Self::Issues(records) => records.push(record),
        }
    }

    pub fn finding(&self, draft: &DraftAdvisory) -> Option<&Value> {
        let (records, field) = match self {
            Self::Advisories(records) => (records, "description"),
            Self::Issues(records) => (records, "body"),
        };
        let marker = format!("Fingerprint: `{}`", draft.fingerprint);
        records.iter().find(|record| {
            record[field]
                .as_str()
                .is_some_and(|text| text.lines().any(|line| line == marker))
        })
    }

    pub fn record(&self, value: &Value) -> Result<SecurityRecord> {
        Ok(match self {
            Self::Advisories(_) => SecurityRecord::Advisory {
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
            Self::Issues(_) => SecurityRecord::Issue {
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
    let (_, reproduction) = description
        .as_str()?
        .split_once("\n<!-- thirdshift:security-reproduction -->\n")?;
    let outcome = reproduction
        .lines()
        .find_map(|line| line.strip_prefix("Outcome: "))?;
    match outcome.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["reproduced", severity, "single" | "spec"] => Severity::parse(severity),
        _ => None,
    }
}

fn has_fix_ticket(description: &Value) -> bool {
    description
        .as_str()
        .is_some_and(|text| text.contains(FIX_TICKET_MARKER))
}

/// A finding read from the private storage selected for this repository.
pub enum SecurityRecord {
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

impl SecurityRecord {
    /// A Day-shift decision is the finding's grade; a repeated fingerprint
    /// must not publish new proof-of-concept evidence or replace that grade.
    pub fn untriaged(&self) -> bool {
        match self {
            Self::Advisory { untriaged, .. } | Self::Issue { untriaged, .. } => *untriaged,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            Self::Advisory { description, .. } | Self::Issue { description, .. } => description,
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Advisory { id, .. } => id.clone(),
            Self::Issue { number, .. } => format!("issue #{number}"),
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
    /// Link the dispatched Ticket without changing the private record's grade.
    pub fn link_security_fix(
        &self,
        repo: &str,
        record: &SecurityRecord,
        ticket: &IssueUrl,
    ) -> Result<()> {
        let (path, field) = match record {
            SecurityRecord::Advisory { id, .. } => (
                format!("repos/{repo}/security-advisories/{id}"),
                "description",
            ),
            SecurityRecord::Issue { number, .. } => {
                (format!("repos/{repo}/issues/{number}"), "body")
            }
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
        if current[field].as_str() != Some(record.description()) {
            bail!("the private record changed while publishing its fix; leaving it unchanged");
        }
        let description = format!(
            "{}{FIX_TICKET_MARKER}{}\n",
            record.description().trim_end(),
            ticket.url
        );
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

    pub fn security_fix_text(&self, ticket: &IssueUrl) -> Result<String> {
        let value = self.issue_view(ticket, "title,body")?;
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
            Ok(records) => Ok(SecurityRecords::Advisories(records)),
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
                Ok(SecurityRecords::Issues(
                    records
                        .into_iter()
                        .filter(|record| record.get("pull_request").is_none())
                        .collect(),
                ))
            }
            Err(error) => Err(error),
        };
        let mut records = records?;
        let (values, field) = match &mut records {
            SecurityRecords::Advisories(values) => (values, "description"),
            SecurityRecords::Issues(values) => (values, "body"),
        };
        for record in values {
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
    ) -> Result<Value> {
        match records {
            SecurityRecords::Advisories(_) => self.create_security_advisory(repo, draft),
            SecurityRecords::Issues(_) => {
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
        }
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
        let (path, storage) = match record {
            SecurityRecord::Advisory { id, .. } => (
                format!("repos/{repo}/security-advisories/{id}"),
                SecurityRecords::Advisories(Vec::new()),
            ),
            SecurityRecord::Issue { number, .. } => (
                format!("repos/{repo}/issues/{number}"),
                SecurityRecords::Issues(Vec::new()),
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
        let current = storage.record(&observed)?;
        if !current.untriaged() {
            bail!(
                "the Security finding was triaged during its reproduction; leaving the record unchanged"
            );
        }
        if current.description() != record.description() {
            bail!(
                "the Security finding changed during its reproduction; leaving the record unchanged"
            );
        }
        if matches!(record, SecurityRecord::Advisory { .. })
            && matches!(reproduction.severity(), Some(Severity::Informational))
        {
            bail!(
                "informational severity has no GitHub advisory field; the Day shift must decide its representation; leaving the record unchanged"
            );
        }
        let description = reproduction.description(record.description());
        let body = match record {
            SecurityRecord::Advisory { .. } => {
                json!({"description": description, "severity": reproduction.severity()})
            }
            SecurityRecord::Issue { .. } => json!({"body": description}),
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
