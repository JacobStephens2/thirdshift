//! Bounded, process-local caches of retained facts. Liveness and health are
//! sampled afresh; only file contents/parsed records survive between requests.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};

use crate::logs::command_record::CommandTree;
use crate::logs::session_signals::SessionSignals;

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
    bytes: u64,
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

    fn read(
        &mut self,
        path: &Path,
        load: impl FnOnce(&Path) -> Result<T>,
        retained_bytes: impl FnOnce(&T, u64) -> u64,
    ) -> Result<T> {
        let identity = Identity::read(path)?;
        if let Some(value) = self.get(path, &identity) {
            return Ok(value);
        }
        let value = load(path)?;
        let bytes = retained_bytes(&value, identity.len);
        // Don't retain a partial in-place write. A later request retries it.
        if Identity::read(path)? == identity {
            self.insert(path, identity, value.clone(), bytes);
        }
        Ok(value)
    }

    fn get(&mut self, path: &Path, identity: &Identity) -> Option<T> {
        if let Some(retained) = self.entries.get_mut(path)
            && retained.identity == *identity
        {
            retained.used = Instant::now();
            return Some(retained.value.clone());
        }
        if let Some(old) = self.entries.remove(path) {
            self.bytes -= old.bytes;
        }
        None
    }

    fn insert(&mut self, path: &Path, identity: Identity, value: T, bytes: u64) {
        const BUDGET: u64 = 64 * 1024 * 1024;
        if bytes > BUDGET {
            return;
        }
        if let Some(old) = self.entries.remove(path) {
            self.bytes -= old.bytes;
        }
        while self.bytes + bytes > BUDGET || self.entries.len() >= 8192 {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(path, _)| path.clone())
                .expect("nonempty cache over budget");
            self.bytes -= self.entries.remove(&oldest).unwrap().bytes;
        }
        self.bytes += bytes;
        self.entries.insert(
            path.to_path_buf(),
            Retained {
                identity,
                value,
                used: Instant::now(),
                bytes,
            },
        );
    }
}

pub(super) fn record(path: &Path) -> Result<Option<CommandTree>> {
    static RECORDS: LazyLock<Mutex<Cache<Option<CommandTree>>>> =
        LazyLock::new(|| Mutex::new(Cache::new()));
    RECORDS
        .lock()
        .unwrap()
        .read(
            path,
            |path| Ok(serde_json::from_slice(&fs::read(path)?).ok()),
            |_, source_bytes| source_bytes,
        )
        .with_context(|| format!("could not read {}", path.display()))
}

pub(super) fn text(path: &Path) -> Result<Arc<str>> {
    static TEXT: LazyLock<Mutex<Cache<Arc<str>>>> = LazyLock::new(|| Mutex::new(Cache::new()));
    match TEXT.lock().unwrap().read(
        path,
        |path| Ok(fs::read_to_string(path)?.into()),
        |_, source_bytes| source_bytes,
    ) {
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

/// Retain only the derived signals, keyed by the resolved Session trace's
/// identity. Large source traces must not consume the summary cache's budget.
pub(super) fn session_signals(log: &Path) -> Option<SessionSignals> {
    struct Signals {
        cache: Cache<Option<SessionSignals>>,
        readers: BTreeMap<PathBuf, Weak<Mutex<()>>>,
    }
    static SIGNALS: LazyLock<Mutex<Signals>> = LazyLock::new(|| {
        Mutex::new(Signals {
            cache: Cache::new(),
            readers: BTreeMap::new(),
        })
    });
    let path = crate::logs::session::path(log);
    let identity = Identity::read(&path).ok()?;
    let reader = {
        let mut signals = SIGNALS.lock().unwrap();
        if let Some(value) = signals.cache.get(&path, &identity) {
            return value;
        }
        // Coordinate readers of this trace only. Parsing a cold trace must not
        // block cached details for unrelated Commands. Weak entries disappear
        // on the next miss once their readers have finished.
        signals
            .readers
            .retain(|_, reader| reader.strong_count() > 0);
        match signals.readers.get(&path).and_then(Weak::upgrade) {
            Some(reader) => reader,
            None => {
                let reader = Arc::new(Mutex::new(()));
                signals
                    .readers
                    .insert(path.clone(), Arc::downgrade(&reader));
                reader
            }
        }
    };
    let _reading = reader.lock().unwrap();
    // A preceding reader or writer may have changed the facts while we waited.
    let identity = Identity::read(&path).ok()?;
    if let Some(value) = SIGNALS.lock().unwrap().cache.get(&path, &identity) {
        return value;
    }
    let value = crate::logs::session_signals::read(&path).ok()?;
    if Identity::read(&path).ok().as_ref() == Some(&identity) {
        let bytes = serde_json::to_vec(&value)
            .expect("Session signals serialize")
            .len() as u64;
        SIGNALS
            .lock()
            .unwrap()
            .cache
            .insert(&path, identity, value.clone(), bytes);
    }
    value
}
