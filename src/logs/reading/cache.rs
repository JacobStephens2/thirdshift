//! Bounded, process-local caches of retained facts. Liveness and health are
//! sampled afresh; only file contents/parsed records survive between requests.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};

use crate::logs::command_record::CommandTree;

#[derive(PartialEq)]
struct Identity {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64, i64, i64),
}

impl Identity {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        })
    }
}

struct Retained<T> {
    identity: Identity,
    value: T,
    used: Instant,
}

struct Cache<T> {
    entries: BTreeMap<PathBuf, Retained<T>>,
    bytes: u64,
}

impl<T: Clone> Cache<T> {
    const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
        }
    }

    fn read(&mut self, path: &Path, load: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
        let identity = Identity::read(path)?;
        if let Some(retained) = self.entries.get_mut(path)
            && retained.identity == identity
        {
            retained.used = Instant::now();
            return Ok(retained.value.clone());
        }
        if let Some(old) = self.entries.remove(path) {
            self.bytes -= old.identity.len;
        }
        let value = load(path)?;
        // Don't retain a partial in-place write. A later request retries it.
        const BUDGET: u64 = 64 * 1024 * 1024;
        if identity.len <= BUDGET && Identity::read(path)? == identity {
            while self.bytes + identity.len > BUDGET || self.entries.len() >= 8192 {
                let oldest = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(path, _)| path.clone())
                    .expect("nonempty cache over budget");
                self.bytes -= self.entries.remove(&oldest).unwrap().identity.len;
            }
            self.bytes += identity.len;
            self.entries.insert(
                path.to_path_buf(),
                Retained {
                    identity,
                    value: value.clone(),
                    used: Instant::now(),
                },
            );
        }
        Ok(value)
    }
}

pub(super) fn record(path: &Path) -> Result<Option<CommandTree>> {
    static RECORDS: LazyLock<Mutex<Cache<Option<CommandTree>>>> =
        LazyLock::new(|| Mutex::new(Cache::new()));
    RECORDS
        .lock()
        .unwrap()
        .read(path, |path| {
            Ok(serde_json::from_slice(&fs::read(path)?).ok())
        })
        .with_context(|| format!("could not read {}", path.display()))
}

pub(super) fn text(path: &Path) -> Result<Arc<str>> {
    static TEXT: LazyLock<Mutex<Cache<Arc<str>>>> = LazyLock::new(|| Mutex::new(Cache::new()));
    match TEXT
        .lock()
        .unwrap()
        .read(path, |path| Ok(fs::read_to_string(path)?.into()))
    {
        Ok(text) => Ok(text),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok("".into())
        }
        Err(error) => Err(error).with_context(|| format!("could not read {}", path.display())),
    }
}
