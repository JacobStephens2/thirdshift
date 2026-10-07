//! Stateless Activity log formatting and file effects. The recording owner
//! decides starts, endings, shared skip history and warning suppression.

use std::fmt::{self, Display};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};

use super::effects::Tail;

pub(super) const FILE: &str = "activity.log";
pub(super) const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub(super) const TAIL: u64 = 64 * 1024;
const TIME_FORMAT_LENGTH: usize = "2026-10-03 10:41:01".len();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Run,
    SpecRun,
    ArchitectRun,
    PickupRun,
}

impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Kind::Run => "Run",
            Kind::SpecRun => "Spec run",
            Kind::ArchitectRun => "Architect run",
            Kind::PickupRun => "Pickup run",
        })
    }
}

pub(super) fn open(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))
}

/// Raw bytes at the end of the file. The owner discards a cut first line.
pub(super) fn tail(path: &Path, limit: u64) -> Result<Tail> {
    let mut file = open(path)?;
    let mut read = || -> std::io::Result<Tail> {
        let from = file.metadata()?.len().saturating_sub(limit);
        file.seek(SeekFrom::Start(from))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(Tail { from, bytes })
    };
    read().with_context(|| format!("could not read {}", path.display()))
}

/// The last line of `kind` in whole lines from the tail, without its time.
pub(super) fn last_of(tail: &str, kind: Kind) -> Option<&str> {
    let kind = kind.to_string();
    tail.lines().rev().find_map(|line| {
        let unstamped = line.get(TIME_FORMAT_LENGTH + 1..)?;
        let rest = unstamped.strip_prefix(&kind)?;
        (rest.starts_with(' ') || rest.starts_with(':')).then_some(unstamped)
    })
}
