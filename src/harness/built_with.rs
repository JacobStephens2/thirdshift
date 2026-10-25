//! The retained Built-with format, shared by PR/notification writers and
//! Scoring's reader. Accept both legacy and version-bearing Harness names.
use std::fmt;
use std::str::FromStr;

use super::{Choice, Harness};

pub(crate) const MARKER: &str = "<!-- thirdshift:built-with -->";
const PREFIX: &str = "Built with ";
const SEPARATOR: char = '·';

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BuiltWith {
    pub harness: Harness,
    pub version: Option<String>,
    pub model: String,
    pub effort: String,
}

pub(super) fn describe(choice: &Choice, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
        f,
        "{} {SEPARATOR} {} {SEPARATOR} {}",
        choice.harness.name(),
        choice.model.as_deref().unwrap_or("default model"),
        choice.effort.as_deref().unwrap_or("default effort")
    )
}

pub(crate) fn read(body: &str) -> Option<BuiltWith> {
    let line = body
        .lines()
        .find_map(|line| line.trim().strip_suffix(MARKER))?;
    line.parse().ok()
}

impl BuiltWith {
    pub fn new(choice: &Choice, version: &str) -> Self {
        Self {
            harness: choice.harness,
            version: Some(version.into()),
            model: choice.model.as_deref().unwrap_or("default model").into(),
            effort: choice.effort.as_deref().unwrap_or("default effort").into(),
        }
    }
}

impl fmt::Display for BuiltWith {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.harness.name())?;
        if let Some(version) = &self.version {
            write!(f, " {version}")?;
        }
        write!(f, " {SEPARATOR} {} {SEPARATOR} {}", self.model, self.effort)
    }
}

impl FromStr for BuiltWith {
    type Err = &'static str;

    /// Both historical lines and versioned lines identify factory work. The
    /// optional ownership marker is not part of the configured Effort.
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let line = line.trim();
        if line.contains(['\r', '\n']) {
            return Err("expected one Built-with line");
        }
        let text = line
            .strip_suffix(MARKER)
            .unwrap_or(line)
            .trim_end()
            .strip_prefix(PREFIX)
            .ok_or("missing Built-with prefix")?;
        let parts: Vec<_> = text.split(SEPARATOR).map(str::trim).collect();
        let [harness, model, effort] = parts.as_slice() else {
            return Err("expected Harness, Model and Effort");
        };
        let mut harness = harness.split_whitespace();
        let name =
            Harness::named(harness.next().ok_or("missing Harness")?).ok_or("unknown Harness")?;
        let version = harness.collect::<Vec<_>>().join(" ");
        if model.is_empty() || effort.is_empty() {
            return Err("empty Built-with field");
        }
        Ok(Self {
            harness: name,
            version: (!version.is_empty()).then_some(version),
            model: (*model).into(),
            effort: (*effort).into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versioned_and_legacy_lines() {
        for (line, harness, version, model, effort) in [
            (
                "Built with claude 2.1.11 · opus · high <!-- thirdshift:built-with -->",
                Harness::Claude,
                Some("2.1.11"),
                "opus",
                "high",
            ),
            (
                "Built with claude · default model · default effort <!-- thirdshift:built-with -->",
                Harness::Claude,
                None,
                "default model",
                "default effort",
            ),
            (
                "Built with codex codex-cli 0.160.0 · gpt-6.1-sol · max",
                Harness::Codex,
                Some("codex-cli 0.160.0"),
                "gpt-6.1-sol",
                "max",
            ),
            (
                "Built with codex · gpt-6.1-sol · max",
                Harness::Codex,
                None,
                "gpt-6.1-sol",
                "max",
            ),
        ] {
            let read: BuiltWith = line.parse().unwrap();
            assert_eq!(read.harness, harness);
            assert_eq!(read.version.as_deref(), version);
            assert_eq!(read.model, model);
            assert_eq!(read.effort, effort);
        }
    }
}
