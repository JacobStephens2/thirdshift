//! The retained Built-with format, shared by PR/notification writers and
//! Scoring's reader. Accept both legacy and version-bearing Harness names.
use std::fmt;

use super::{Choice, Harness};

pub(crate) const MARKER: &str = "<!-- thirdshift:built-with -->";
const PREFIX: &str = "Built with ";
const SEPARATOR: char = '·';

pub(crate) struct Attribution {
    pub harness: Harness,
    pub harness_version: Option<String>,
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

pub(super) fn line(choice: &Choice) -> String {
    format!("{PREFIX}{choice}")
}

pub(crate) fn read(body: &str) -> Option<Attribution> {
    let line = body
        .lines()
        .find_map(|line| line.trim().strip_suffix(MARKER))?;
    let mut parts = line
        .trim()
        .strip_prefix(PREFIX)?
        .split(SEPARATOR)
        .map(str::trim);
    let mut harness = parts.next()?.split_whitespace();
    let name = Harness::named(harness.next()?)?;
    let version = harness.collect::<Vec<_>>().join(" ");
    let model = parts.next()?.to_string();
    let effort = parts.next()?.to_string();
    if parts.next().is_some() || model.is_empty() || effort.is_empty() {
        return None;
    }
    Some(Attribution {
        harness: name,
        harness_version: (!version.is_empty()).then_some(version),
        model,
        effort,
    })
}
