//! The latest observed start of each Pass, independent of skip history.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local};
use serde::Serialize;

use super::{Pass, reading, status};

#[derive(Serialize)]
pub(crate) struct PassStart {
    pub repository: String,
    pub kind: String,
    pub last_fired_at: DateTime<FixedOffset>,
    pub age_seconds: i64,
}

impl PassStart {
    pub fn line(&self) -> String {
        format!(
            "{} last fired {} ago on {}",
            self.kind.strip_suffix(" run").unwrap_or(&self.kind),
            status::age(self.age_seconds),
            self.repository
        )
    }
}

pub(crate) fn load(root: &Path) -> Result<Vec<PassStart>> {
    reading::pass_starts(root, Local::now())
}

pub(super) fn path(root: &Path, pass: Pass) -> PathBuf {
    let kind = match pass {
        Pass::Architect => "architect",
        Pass::Pickup => "pickup",
        Pass::Security => "secure",
    };
    root.join(format!("{kind}.last-fired"))
}

pub(super) fn read(root: &Path, repository: &str, now: DateTime<Local>) -> Result<Vec<PassStart>> {
    let mut starts = Vec::new();
    for pass in [Pass::Architect, Pass::Pickup, Pass::Security] {
        let path = path(root, pass);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("could not read {}", path.display()));
            }
        };
        // A damaged time is no evidence that a Pass fired.
        if let Ok(last_fired_at) = DateTime::parse_from_rfc3339(text.trim()) {
            starts.push(PassStart {
                repository: repository.into(),
                kind: pass.to_string(),
                last_fired_at,
                age_seconds: (now.fixed_offset() - last_fired_at).num_seconds().max(0),
            });
        }
    }
    Ok(starts)
}
