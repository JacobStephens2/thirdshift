//! Per-Pass author exclusions. These only narrow the fixed trust rule.

use anyhow::Result;

use crate::github::GitHub;
use crate::issue::IssueUrl;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SkipAuthors(pub Vec<String>);

impl SkipAuthors {
    /// Why this Pass excludes `issue`, if its author matches a named login.
    /// With no exclusions, do not add a GitHub read to ordinary commands.
    pub fn reason(&self, github: &GitHub, issue: &IssueUrl) -> Result<Option<String>> {
        if self.0.is_empty() {
            return Ok(None);
        }
        let Some(login) = github.issue_author(issue)? else {
            return Ok(None);
        };
        Ok(self
            .0
            .iter()
            .any(|skipped| skipped.eq_ignore_ascii_case(&login))
            .then(|| format!("written by {login}, whom this Pass skips")))
    }
}
