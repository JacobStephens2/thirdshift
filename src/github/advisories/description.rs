//! The persisted private description protocol, inspected only as needed.

use anyhow::{Context, Result, bail};

use super::{FindingDraft, FindingProvenance, FixEnding};
use crate::issue::IssueUrl;
use crate::security::reproduction::{FixSize, Outcome, Reproduction, Severity};

const FIX_TICKET_MARKER: &str = "\n<!-- thirdshift:security-fix -->\nFix Ticket: ";
const REPRODUCTION_MARKER: &str = "\n<!-- thirdshift:security-reproduction -->\n";

pub(super) struct Description<'a>(pub(super) &'a str);

enum FixStatus {
    Pending,
    Failed,
    Succeeded,
}

impl<'a> Description<'a> {
    pub(super) fn draft(input: &FindingDraft<'_>) -> String {
        let (source, review) = match input.provenance {
            FindingProvenance::Audit => ("run", String::new()),
            FindingProvenance::Review { issue_url } => {
                ("review", format!("Review issue: {issue_url}\n"))
            }
        };
        format!(
            "Found by thirdshift's Security {source}.\n\nFingerprint: `{}`\nAudited commit: `{}`\n{review}\n{}\n\n```json\n{}\n```\n",
            input.fingerprint, input.audited_commit, input.original_description, input.evidence
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
    pub(super) fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        let marker = format!("Fingerprint: `{fingerprint}`");
        self.0.lines().any(|line| line == marker)
    }

    pub(super) fn reproduced_outcome(&self) -> Option<(Severity, FixSize)> {
        let (_, reproduction) = self.0.split_once(REPRODUCTION_MARKER)?;
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

    pub(super) fn with_reproduction(&self, reproduction: &Reproduction) -> String {
        let original = self.0;
        // Replacing reproduction evidence must keep the dispatched fix's link.
        let (original, fix) = original
            .rsplit_once(FIX_TICKET_MARKER)
            .map_or((original, String::new()), |(original, ticket)| {
                (original, format!("{FIX_TICKET_MARKER}{ticket}"))
            });
        let original = original
            .split_once(REPRODUCTION_MARKER)
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

    fn fix_section(&self) -> Option<&'a str> {
        self.0.rsplit_once(FIX_TICKET_MARKER).map(|(_, fix)| fix)
    }

    pub(super) fn fix_link(&self) -> Option<&'a str> {
        self.fix_section()
            .map(|fix| fix.lines().next().unwrap_or_default())
    }

    fn fix_status(&self) -> Option<FixStatus> {
        match self
            .fix_section()?
            .lines()
            .rev()
            .find(|line| line.starts_with("Fix Run: "))?
        {
            "Fix Run: pending" => Some(FixStatus::Pending),
            "Fix Run: failed" => Some(FixStatus::Failed),
            "Fix Run: succeeded" => Some(FixStatus::Succeeded),
            _ => None,
        }
    }

    pub(super) fn failed_fix(&self) -> Result<Option<IssueUrl>> {
        // The Pass lock rules out an active dispatch here. Pending also means
        // failed/incomplete until a successful ending is recorded.
        if matches!(
            self.fix_status(),
            Some(FixStatus::Pending | FixStatus::Failed)
        ) {
            return Ok(Some(IssueUrl::parse(self.fix_link().unwrap_or_default())?));
        }
        Ok(None)
    }

    pub(super) fn with_pending_fix(&self, captured: &str, issue: &IssueUrl) -> Result<String> {
        if self.0 != captured {
            bail!("the private record changed while publishing its fix; leaving it unchanged");
        }
        // Persist before dispatch: a later write failure cannot reopen Fencing.
        Ok(format!(
            "{}{FIX_TICKET_MARKER}{}\nFix Run: pending\n",
            self.0.trim_end(),
            issue.url
        ))
    }

    pub(super) fn with_fix_ending(&self, issue: &IssueUrl, ending: FixEnding) -> Result<String> {
        if self.fix_link() != Some(issue.url.as_str()) {
            bail!("the private record's fix Ticket changed during its Run; leaving it unchanged");
        }
        let ending = match ending {
            FixEnding::Failed => "Fix Run: failed",
            FixEnding::Succeeded => "Fix Run: succeeded",
        };
        Ok(format!("{}\n{ending}\n", self.0.trim_end()))
    }
    pub(super) fn check_public_fix_text(&self, body: &str, url: &str) -> Result<()> {
        if !body.contains(url) {
            bail!("the Security fix Ticket does not link the private record");
        }
        // Reject copied write-up lines and test text. The session is also
        // instructed to publish only what the fix changes, never a paraphrase.
        for line in self.0.lines().filter(|line| {
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
        }) {
            if body.contains(line.trim()) {
                bail!("the Security fix Ticket includes private write-up text");
            }
        }
        Ok(())
    }
}
