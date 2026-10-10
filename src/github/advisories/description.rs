//! Persistent description conventions, private to the Security record module.

use anyhow::{Context, Result, bail};

use super::FindingSource;
use crate::issue::IssueUrl;
use crate::security::reproduction::{FixSize, Outcome, Reproduction, Severity};

const FIX_TICKET_MARKER: &str = "\n<!-- thirdshift:security-fix -->\nFix Ticket: ";

const FIX_PENDING: &str = "Fix Run: pending";
const FIX_FAILED: &str = "Fix Run: failed";
const FIX_SUCCEEDED: &str = "Fix Run: succeeded";

#[derive(Clone, Copy)]
pub(super) enum FixEnding {
    Pending,
    Failed,
    Succeeded,
}

impl FixEnding {
    fn line(self) -> &'static str {
        match self {
            Self::Pending => FIX_PENDING,
            Self::Failed => FIX_FAILED,
            Self::Succeeded => FIX_SUCCEEDED,
        }
    }
}

pub(super) fn render_finding(
    source: FindingSource<'_>,
    fingerprint: &str,
    write_up: &str,
    evidence: &str,
) -> String {
    let (name, commit, review) = match source {
        FindingSource::Audit { commit } => ("run", commit, String::new()),
        FindingSource::Review { commit, issue } => {
            ("review", commit, format!("Review issue: {}\n", issue.url))
        }
    };
    format!(
        "Found by thirdshift's Security {name}.\n\nFingerprint: `{fingerprint}`\nAudited commit: `{commit}`\n{review}\n{write_up}\n\n```json\n{evidence}\n```\n"
    )
}

pub(super) struct Description<'a>(pub(super) &'a str);

impl<'a> Description<'a> {
    pub(super) fn contains_private_text(&self, public_text: &str) -> bool {
        // Reject copied write-up lines and test text. Publishing also instructs
        // the session to describe only the fix, never a paraphrase of the finding.
        self.0
            .lines()
            .filter(|line| {
                let line = line.trim();
                // A bare identifier can also be an ordinary word in fix prose.
                // Complete short statements such as bypass_login(); stay checked.
                line.chars().any(char::is_alphanumeric)
                    && !line.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
                    && !line.starts_with("```")
                    && !line.starts_with("Fingerprint:")
                    && !line.starts_with("Audited commit:")
                    && !line.starts_with("Outcome:")
                    && !line.starts_with("Severity:")
                    && !line.starts_with("Fix size:")
                    && !line.starts_with('#')
                    && !line.starts_with("<!--")
            })
            .any(|line| public_text.contains(line.trim()))
    }

    pub(super) fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        let marker = format!("Fingerprint: `{fingerprint}`");
        self.0.lines().any(|line| line == marker)
    }

    pub(super) fn reproduced_outcome(&self) -> Option<(Severity, FixSize)> {
        let (_, reproduction) = self
            .0
            .split_once("\n<!-- thirdshift:security-reproduction -->\n")?;
        let outcome = reproduction
            .lines()
            .find_map(|line| line.strip_prefix("Outcome: "))?;
        match outcome.split_whitespace().collect::<Vec<_>>().as_slice() {
            ["reproduced", severity, "single"] => {
                Some((Severity::parse(severity)?, FixSize::Single))
            }
            ["reproduced", severity, "spec"] => Some((Severity::parse(severity)?, FixSize::Spec)),
            _ => None,
        }
    }

    pub(super) fn has_fix(&self) -> bool {
        self.0.contains(FIX_TICKET_MARKER)
    }

    fn fix_suffix(&self) -> Option<&'a str> {
        self.0
            .rsplit_once(FIX_TICKET_MARKER)
            .map(|(_, suffix)| suffix)
    }

    pub(super) fn fix_ticket(&self) -> Option<&'a str> {
        Some(self.fix_suffix()?.lines().next().unwrap_or_default())
    }

    pub(super) fn fix_ending(&self) -> Option<FixEnding> {
        match self
            .fix_suffix()?
            .lines()
            .rev()
            .find(|line| line.starts_with("Fix Run: "))?
        {
            FIX_PENDING => Some(FixEnding::Pending),
            FIX_FAILED => Some(FixEnding::Failed),
            FIX_SUCCEEDED => Some(FixEnding::Succeeded),
            _ => None,
        }
    }

    pub(super) fn link_fix(&self, issue: &IssueUrl) -> String {
        format!(
            "{}{FIX_TICKET_MARKER}{}\n{}\n",
            self.0.trim_end(),
            issue.url,
            FixEnding::Pending.line()
        )
    }

    pub(super) fn complete_fix(&self, issue: &IssueUrl, succeeded: bool) -> Result<String> {
        if self.fix_ticket() != Some(issue.url.as_str()) {
            bail!("the private record's fix Ticket changed during its Run; leaving it unchanged");
        }
        let ending = if succeeded {
            FixEnding::Succeeded
        } else {
            FixEnding::Failed
        };
        Ok(format!("{}\n{}\n", self.0.trim_end(), ending.line()))
    }

    pub(super) fn with_reproduction(&self, reproduction: &Reproduction) -> String {
        let original = self.0;
        // Replacing reproduction evidence must keep the dispatched fix's link.
        let (original, fix) = original
            .rsplit_once(FIX_TICKET_MARKER)
            .map_or((original, String::new()), |(original, ticket)| {
                (original, format!("{FIX_TICKET_MARKER}{ticket}"))
            });
        let original = original
            .split_once("\n<!-- thirdshift:security-reproduction -->\n")
            .map_or(original, |(original, _)| original);
        let severity = reproduction
            .severity()
            .map(|severity| format!("Severity: {}\n", severity.name()))
            .unwrap_or_default();
        let size = match &reproduction.outcome {
            Outcome::Reproduced { size, .. } => format!("Fix size: {}\n", size.name()),
            Outcome::NotReproduced => String::new(),
        };
        // A test may itself contain Markdown fences. Preserve its text without
        // allowing one of those fences to close the record's code block.
        let fence = "`".repeat(
            reproduction
                .test
                .lines()
                .map(|line| line.chars().take_while(|c| *c == '`').count())
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        format!(
            "{}\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: {}\n{severity}{size}\n{}\n\n### Proof-of-concept test\n\n{fence}\n{}{fence}\n{fix}",
            original.trim_end(),
            reproduction.outcome,
            reproduction.notes,
            if reproduction.test.ends_with('\n') {
                reproduction.test.clone()
            } else {
                format!("{}\n", reproduction.test)
            }
        )
    }

    pub(super) fn audited_commit(&self) -> Result<&'a str> {
        let commit = self
            .0
            .lines()
            .find_map(|line| {
                line.strip_prefix("Audited commit: `")
                    .and_then(|commit| commit.strip_suffix('`'))
            })
            .context("Security finding record has no audited commit")?;
        if ![40, 64].contains(&commit.len()) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("Security finding record has an invalid audited commit");
        }
        Ok(commit)
    }
}
