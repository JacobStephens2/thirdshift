//! Private advisory operations. Publishing and closing belong to the Day shift.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Value, json};

use super::GitHub;

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
    pub fn already_recorded(&self, advisories: &[Value]) -> bool {
        let marker = format!("Fingerprint: `{}`", self.fingerprint);
        advisories.iter().any(|advisory| {
            advisory["description"]
                .as_str()
                .is_some_and(|description| description.lines().any(|line| line == marker))
        })
    }
}

impl GitHub {
    /// Every state and every page, including closed and published records.
    pub fn security_advisories(&self, repo: &str) -> Result<Vec<Value>> {
        self.gh_api_items(
            &format!("repos/{repo}/security-advisories?per_page=100"),
            "",
        )
    }

    /// The create endpoint creates a draft. No severity or version claim.
    pub fn create_security_advisory(&self, repo: &str, draft: &DraftAdvisory) -> Result<Value> {
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
