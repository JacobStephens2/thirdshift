//! Counts from Delivery's last published Unaddressed findings section.

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct UnaddressedFindings {
    standards: usize,
    spec: usize,
    security: usize,
}

enum Axis {
    Standards,
    Spec,
    Security,
    SecurityOutcome,
}

impl Axis {
    fn named(text: &str) -> Option<Self> {
        match label(text).to_ascii_lowercase().as_str() {
            "standards" => Some(Self::Standards),
            "spec" => Some(Self::Spec),
            "security" => Some(Self::Security),
            "security review outcome" => Some(Self::SecurityOutcome),
            _ => None,
        }
    }
}

fn label(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '*' | '_' | ':' | '.' | '#'))
}

fn list_item(text: &str) -> Option<&str> {
    text.strip_prefix("- ")
        .or_else(|| text.strip_prefix("* "))
        .or_else(|| text.strip_prefix("+ "))
        .or_else(|| {
            let end = text.find(|c: char| !c.is_ascii_digit())?;
            if end == 0 {
                return None;
            }
            text[end..]
                .strip_prefix(". ")
                .or_else(|| text[end..].strip_prefix(") "))
        })
}

impl UnaddressedFindings {
    pub(super) fn include_security_review(&mut self, count: usize) {
        self.security = self.security.max(count);
    }

    pub(super) fn from_body(body: &str) -> Self {
        let mut counts = Self::default();
        let mut section_level = None;
        let mut axis = None;
        let mut item_indent = None;
        let mut security_outcome = 0;
        let mut fence = None;
        for line in body.lines() {
            let text = line.trim();
            if text.starts_with("```") || text.starts_with("~~~") {
                let marker = &text[..3];
                if fence == Some(marker) {
                    fence = None;
                } else if fence.is_none() {
                    fence = Some(marker);
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            let title = text.trim_start_matches('#');
            let level = text.len() - title.len();
            if level > 0 && title.starts_with(' ') {
                if label(title).eq_ignore_ascii_case("Unaddressed findings") {
                    section_level = Some(level);
                    axis = None;
                } else if let Some(section) = section_level {
                    if level <= section {
                        section_level = None;
                    }
                    axis = Axis::named(title);
                }
                item_indent = None;
                continue;
            }
            if section_level.is_none() {
                continue;
            }
            let item = list_item(text);
            if let Some(named) = Axis::named(item.unwrap_or(text)) {
                axis = Some(named);
                item_indent = None;
                continue;
            }
            let Some(item) = item else { continue };
            if matches!(
                label(item).to_ascii_lowercase().as_str(),
                "" | "none"
                    | "no findings"
                    | "no unaddressed findings"
                    | "no unaddressed introduced findings"
            ) {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let root = *item_indent.get_or_insert(indent);
            if indent > root {
                continue;
            }
            match axis {
                Some(Axis::Standards) => counts.standards += 1,
                Some(Axis::Spec) => counts.spec += 1,
                Some(Axis::Security) => counts.security += 1,
                Some(Axis::SecurityOutcome) => security_outcome += 1,
                None => {}
            }
        }
        // Delivery's deterministic summary repeats the review's detailed entries.
        counts.security = counts.security.max(security_outcome);
        counts
    }
}
