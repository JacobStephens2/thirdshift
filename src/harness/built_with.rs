//! The pull request's attribution, also used by the Run notification.

use std::fmt;
use std::str::FromStr;

use super::{Choice, Harness, version};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BuiltWith {
    pub harness: Harness,
    pub version: Option<String>,
    pub model: String,
    pub effort: String,
}

impl From<&Choice> for BuiltWith {
    fn from(choice: &Choice) -> Self {
        let owner = crate::logs::captured_owner().ok();
        Self {
            harness: choice.harness,
            version: Some(version::detected(
                choice.harness,
                owner.as_ref().and_then(|owner| owner.log_path()),
            )),
            model: choice.model.as_deref().unwrap_or("default model").into(),
            effort: choice.effort.as_deref().unwrap_or("default effort").into(),
        }
    }
}

impl fmt::Display for BuiltWith {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Built with {}", self.harness.name())?;
        if let Some(version) = &self.version {
            write!(f, " {version}")?;
        }
        write!(f, " · {} · {}", self.model, self.effort)
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
            .strip_suffix("<!-- thirdshift:built-with -->")
            .unwrap_or(line)
            .trim_end()
            .strip_prefix("Built with ")
            .ok_or("missing Built-with prefix")?;
        let parts: Vec<_> = text.split(" · ").collect();
        let [harness, model, effort] = parts.as_slice() else {
            return Err("expected Harness, Model and Effort");
        };
        let (harness, version) = harness
            .split_once(' ')
            .map_or((*harness, None), |(harness, version)| {
                (harness, Some(version.trim()))
            });
        if model.trim().is_empty() || effort.trim().is_empty() || version.is_some_and(str::is_empty)
        {
            return Err("empty Built-with field");
        }
        Ok(Self {
            harness: Harness::named(harness).ok_or("unknown Harness")?,
            version: version.map(str::to_owned),
            model: model.trim().into(),
            effort: effort.trim().into(),
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
