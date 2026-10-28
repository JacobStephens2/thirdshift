//! Disposable derived evidence beside a Session trace, never in its Command
//! record. Atomic replacement and a per-trace lock keep CLI readers coordinated.
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::Identity;
use crate::logs::session_signals::SessionSignals;

#[derive(PartialEq, Serialize, Deserialize)]
struct Executable {
    path: PathBuf,
    identity: Identity,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    executable: Executable,
    trace: Identity,
    signals: Option<SessionSignals>,
}

pub(super) struct RetainedSignals {
    path: PathBuf,
    // Locks a separate stable inode, since the saved value is replaced atomically.
    _lock: File,
}

impl RetainedSignals {
    pub(super) fn lock(trace: &Path) -> io::Result<Self> {
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(trace.with_extension("signals-cache.lock"))?;
        lock.lock()?;
        Ok(Self {
            path: trace.with_extension("signals-cache.json"),
            _lock: lock,
        })
    }

    pub(super) fn read(&self, identity: &Identity) -> Option<Option<SessionSignals>> {
        let executable = executable()?;
        let saved: Saved = serde_json::from_slice(&fs::read(&self.path).ok()?).ok()?;
        (saved.trace == *identity && saved.executable == *executable).then_some(saved.signals)
    }

    pub(super) fn write(
        &self,
        identity: &Identity,
        signals: &Option<SessionSignals>,
    ) -> anyhow::Result<()> {
        // Include the exact executable identity so a change to signal semantics
        // cannot silently reuse values derived by a previous binary.
        let Some(executable) = executable() else {
            return Ok(());
        };
        let temporary = tempfile::NamedTempFile::new_in(self.path.parent().unwrap())?;
        serde_json::to_writer(
            &temporary,
            &serde_json::json!({
                "executable": executable, "trace": identity, "signals": signals
            }),
        )?;
        temporary.persist(&self.path)?;
        Ok(())
    }
}

fn executable() -> Option<&'static Executable> {
    static EXECUTABLE: LazyLock<Option<Executable>> = LazyLock::new(|| {
        let path = std::env::current_exe().ok()?;
        let identity = Identity::read(&path).ok()?;
        Some(Executable { path, identity })
    });
    EXECUTABLE.as_ref()
}
