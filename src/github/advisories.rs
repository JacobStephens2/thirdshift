//! Private Security finding records. Public repositories use draft advisories;
//! private repositories whose advisory endpoint is unavailable use issues.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::GitHub;
use crate::issue::IssueUrl;
use crate::labels::{Label, NEEDS_TRIAGE};

const SECURITY_FINDING: Label = Label::new(
    "security-finding",
    "A Security finding recorded privately for the Day shift",
);

pub enum SecurityRecords {
    Advisories(Vec<Value>),
    Issues(Vec<Value>),
}

impl SecurityRecords {
    pub fn remember(&mut self, record: Value) -> &Value {
        let records = match self {
            Self::Advisories(records) | Self::Issues(records) => records,
        };
        records.push(record);
        records.last().expect("the new record was just added")
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

impl DraftAdvisory {
    pub fn recorded_in<'a>(&self, records: &'a SecurityRecords) -> Option<&'a Value> {
        let marker = format!("Fingerprint: `{}`", self.fingerprint);
        let (records, field) = match records {
            SecurityRecords::Advisories(records) => (records, "description"),
            SecurityRecords::Issues(records) => (records, "body"),
        };
        records.iter().find(|record| {
            record[field]
                .as_str()
                .is_some_and(|description| description.lines().any(|line| line == marker))
        })
    }
}

impl GitHub {
    /// Every state and every page. A 404 selects issues only when repository
    /// metadata confirms they will be private.
    pub fn security_records(&self, repo: &str) -> Result<SecurityRecords> {
        match self.gh_api_items(
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
        }
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
                IssueUrl::parse(url.trim()).map_err(|_| {
                    anyhow::anyhow!(
                        "creating a private Security finding issue returned no issue URL"
                    )
                })?;
                Ok(json!({
                    "body": draft.description,
                    "title": draft.summary,
                    "html_url": url.trim()
                }))
            }
        }
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
