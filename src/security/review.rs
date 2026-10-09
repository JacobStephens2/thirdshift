//! The guidance review's final-line contract. Only introduced findings may
//! appear here; pre-existing vulnerability details stay out of public text.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fmt;

use crate::harness::interpretation::SafeguardRefusal;

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
}

impl Review {
    pub fn from_final_message(message: Option<&str>) -> Result<Self> {
        let json = message
            .and_then(|message| {
                message
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .next_back()
            })
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
