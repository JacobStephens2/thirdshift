//! One native filesystem observer for the Console, shared by all streams.
//! Watch directories so atomic replacements keep working. The slow fallback
//! recovers lost notifications and observations of process death or elapsed time.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::watch;

#[derive(Clone, Copy, Default)]
pub(super) struct Revision {
    pub commands: u64,
    pub accounts: u64,
    executable: u64,
}

pub(super) type Receiver = watch::Receiver<Revision>;

#[derive(Clone)]
pub(super) struct Notifier {
    pending: watch::Sender<Revision>,
    following: Arc<Mutex<Weak<super::watching::Following>>>,
}

impl Notifier {
    pub fn commands(&self) {
        self.pending.send_modify(|revision| revision.commands += 1);
    }

    pub fn follow(&self, changes: Receiver) -> Arc<super::watching::Following> {
        let mut current = self.following.lock().unwrap();
        if let Some(following) = current.upgrade() {
            return following;
        }
        let following = Arc::new(super::watching::follow(self.clone(), changes));
        *current = Arc::downgrade(&following);
        following
    }
}

#[derive(PartialEq)]
struct Registration {
    mode: RecursiveMode,
    created: Option<SystemTime>,
    #[cfg(unix)]
    inode: (u64, u64),
}

impl Registration {
    fn at(path: &std::path::Path, mode: RecursiveMode) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            mode,
            created: metadata.created().ok(),
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
        })
    }
}

struct Paths {
    root: PathBuf,
    logs: PathBuf,
    executable: PathBuf,
}

impl Paths {
    fn notice(&self, result: notify::Result<Event>, revision: &mut Revision) {
        let event = match result {
            Ok(event) if matches!(event.kind, EventKind::Access(_)) => return,
            Ok(event) if !event.need_rescan() => event,
            error => {
                if let Err(error) = error {
                    eprintln!("thirdshift Console: filesystem observer: {error}");
                }
                revision.commands += 1;
                revision.accounts += 1;
                revision.executable += 1;
                return;
            }
        };
        for path in event.paths {
            if path == self.root.join("config.toml") || self.root.starts_with(&path) {
                revision.commands += 1;
                revision.accounts += 1;
            }
            if path == self.root.join("console-accounts.json") {
                revision.accounts += 1;
            }
            if path.starts_with(&self.logs)
                || self.logs.starts_with(&path)
                || path.starts_with(self.root.join("console"))
            {
                revision.commands += 1;
            }
            if path == self.executable {
                revision.executable += 1;
            }
        }
    }

    fn watches(&self) -> BTreeMap<PathBuf, RecursiveMode> {
        let mut paths = BTreeMap::new();
        for (path, recursive) in [
            (self.root.clone(), RecursiveMode::NonRecursive),
            (self.root.join("console"), RecursiveMode::Recursive),
            (self.logs.clone(), RecursiveMode::Recursive),
            (
                self.executable.parent().unwrap().to_path_buf(),
                RecursiveMode::NonRecursive,
            ),
        ] {
            // An absent log root is normal. Observe its nearest existing parent
            // until it is created, then move the recursive watch onto the root.
            let ancestor = path.ancestors().find(|path| path.is_dir()).unwrap();
            let mode = if ancestor == path {
                recursive
            } else {
                RecursiveMode::NonRecursive
            };
            paths
                .entry(ancestor.to_path_buf())
                .and_modify(|old| {
                    if mode == RecursiveMode::Recursive {
                        *old = mode;
                    }
                })
                .or_insert(mode);
            // Retain a parent watch when the root itself is replaced.
            if ancestor == path
                && let Some(parent) = path.parent()
            {
                paths
                    .entry(parent.to_path_buf())
                    .or_insert(RecursiveMode::NonRecursive);
            }
        }
        paths
    }
}

pub(super) fn observe(
    config: &crate::config::UserConfig,
    executable: &super::restart::Executable,
) -> Result<(Receiver, Notifier, tokio::task::JoinHandle<()>)> {
    let paths = Arc::new(RwLock::new(Paths {
        root: crate::config::home()?.join(".thirdshift"),
        logs: config.logs_dir.clone(),
        executable: executable.path.clone(),
    }));
    let (pending, mut notifications) = watch::channel(Revision::default());
    let notifier = Notifier {
        pending: pending.clone(),
        following: Arc::default(),
    };
    let observed = paths.clone();
    let mut watcher = notify::recommended_watcher(move |event| {
        pending.send_if_modified(|revision| {
            let before = (revision.commands, revision.accounts, revision.executable);
            observed.read().unwrap().notice(event, revision);
            before != (revision.commands, revision.accounts, revision.executable)
        });
    })?;
    let mut watched = BTreeMap::new();
    reconcile(&mut watcher, &mut watched, &paths.read().unwrap());
    let (sender, changes) = watch::channel(Revision::default());
    let observer =
        tokio::spawn(async move {
            let mut previous = Revision::default();
            loop {
                let fallback =
                    match tokio::time::timeout(Duration::from_secs(60), notifications.changed())
                        .await
                    {
                        Ok(Ok(())) => false,
                        Err(_) => true,
                        Ok(Err(_)) => {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            true
                        }
                    };
                // Coalesce an atomic save and bursts of log appends into one notice.
                if !fallback {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                let revision = *notifications.borrow_and_update();
                if (fallback || revision.accounts != previous.accounts)
                    && let Ok(config) = crate::config::UserConfig::load()
                {
                    paths.write().unwrap().logs = config.logs_dir;
                }
                reconcile(&mut watcher, &mut watched, &paths.read().unwrap());
                sender.send_modify(|current| {
                    if fallback || revision.commands != previous.commands {
                        current.commands += 1;
                    }
                    if fallback || revision.accounts != previous.accounts {
                        current.accounts += 1;
                    }
                    if fallback || revision.executable != previous.executable {
                        current.executable += 1;
                    }
                });
                previous = revision;
            }
        });
    Ok((changes, notifier, observer))
}

fn reconcile(
    watcher: &mut notify::RecommendedWatcher,
    watched: &mut BTreeMap<PathBuf, Registration>,
    paths: &Paths,
) {
    let desired = paths.watches();
    for (path, mode) in &desired {
        let Ok(registration) = Registration::at(path, *mode) else {
            continue;
        };
        if watched.get(path) != Some(&registration) {
            if watched.remove(path).is_some() {
                let _ = watcher.unwatch(path);
            }
            match watcher.watch(path, *mode) {
                Ok(()) => {
                    watched.insert(path.clone(), registration);
                }
                Err(error) => eprintln!(
                    "thirdshift Console: can't watch {} (using 60s fallback): {error}",
                    path.display()
                ),
            }
        }
    }
    watched.retain(|path, _| {
        if desired.contains_key(path) {
            return true;
        }
        let _ = watcher.unwatch(path);
        false
    });
}
