//! Effective activity paths belong to the observed tree. File facts are lazy
//! and shared by its Snapshot; writer lifetime remains a fresh process check.
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use super::Command;

pub(super) struct Activity {
    log: PathBuf,
    session: Option<PathBuf>,
    now: DateTime<Local>,
}

impl Activity {
    pub(super) fn new(log: &Path, session: Option<&Path>, now: DateTime<Local>) -> Self {
        Self {
            log: log.to_owned(),
            session: session.map(crate::logs::session::path),
            now,
        }
    }

    /// Drop a Session path another account named outside its Command record's directory.
    pub(super) fn drop_session_unless(&mut self, keep: impl Fn(&Path) -> bool) {
        if self.session.as_deref().is_some_and(|path| !keep(path)) {
            self.session = None;
        }
    }
}

impl Command {
    /// The owning Command log followed by this record's current Session trace.
    /// These lexical paths may be outside the Log inventory; no metadata is read.
    pub(in crate::logs) fn activity_paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::once(self.activity.log.as_path()).chain(self.activity.session.as_deref())
    }

    /// Quiet time starts at the latest activity or stage entry and excludes
    /// only the suspensions overlapping that interval.
    pub(in crate::logs) fn quiet_age(&self) -> u64 {
        let Some(stage) = self.record.current_stage() else {
            return 0;
        };
        let Ok(began) = DateTime::parse_from_rfc3339(&stage.began_at) else {
            return 0;
        };
        let since = self
            .activity_paths()
            .filter_map(|path| self.inventory.activity(path).modified)
            .map(|modified| DateTime::<Local>::from(modified).fixed_offset())
            .fold(began, std::cmp::max);
        let now = self.activity.now.fixed_offset();
        let quiet = (now - since).num_seconds().max(0) as u64;
        let suspended = stage.suspended_between(since, now);
        quiet.saturating_sub(suspended)
    }

    /// Lengths share the health sample, with zero for an unavailable file.
    pub(in crate::logs) fn activity_lengths(&self) -> (u64, u64) {
        (
            self.inventory.activity(&self.activity.log).length,
            self.activity
                .session
                .as_deref()
                .map_or(0, |path| self.inventory.activity(path).length),
        )
    }
}
