//! One native filesystem observer for the Console, shared by all streams.
//! Watch directories so atomic replacements keep working. The slow fallback
//! recovers lost notifications.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::watch;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) struct Revision {
    pub commands: u64,
    pub accounts: u64,
    pub styles: u64,
    pub scripts: u64,
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
    assets: Option<PathBuf>,
}

impl Paths {
    fn new(root: PathBuf, logs: PathBuf, executable: PathBuf) -> Self {
        Self {
            root: observed_path(&root),
            logs: observed_path(&logs),
            executable: observed_path(&executable),
            assets: None,
        }
    }

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
                if self.assets.is_some() {
                    revision.styles += 1;
                    revision.scripts += 1;
                }
                return;
            }
        };
        for path in event.paths {
            if crate::logs::github::bookkeeping(&path, &self.logs) {
                continue;
            }
            // Parent-directory mtimes change for unrelated children and our
            // own account/log writes. File events carry the actual change.
            if matches!(
                event.kind,
                EventKind::Modify(notify::event::ModifyKind::Metadata(_))
            ) && path.is_dir()
            {
                continue;
            }
            if let Some(directory) = &self.assets
                && (path.starts_with(directory) || directory.starts_with(&path))
            {
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some("js") => revision.scripts += 1,
                    Some("css" | "woff2" | "woff" | "otf" | "ttf") => revision.styles += 1,
                    _ if path == *directory
                        || path.is_dir()
                        || matches!(
                            event.kind,
                            EventKind::Remove(notify::event::RemoveKind::Folder)
                        ) =>
                    {
                        revision.styles += 1;
                        revision.scripts += 1;
                    }
                    _ => {}
                }
            }
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
        ]
        .into_iter()
        .chain(
            self.assets
                .iter()
                .map(|path| (path.clone(), RecursiveMode::Recursive)),
        ) {
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

// Native backends may report canonical paths (notably /private/var on macOS).
// Resolve the existing ancestor so roots created later use the same spelling.
pub(super) fn observed_path(path: &std::path::Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(canonical) = ancestor.canonicalize() {
            return canonical.join(path.strip_prefix(ancestor).unwrap());
        }
    }
    path.to_path_buf()
}

pub(super) fn observe(
    config: &crate::config::UserConfig,
    executable: &super::restart::Executable,
    assets: &super::assets::Development,
) -> Result<(Receiver, Notifier, tokio::task::JoinHandle<()>)> {
    let mut paths = Paths::new(
        crate::config::home()?.join(".thirdshift"),
        config.logs_dir.clone(),
        executable.path.clone(),
    );
    paths.assets = assets.directory().map(PathBuf::from);
    let paths = Arc::new(RwLock::new(paths));
    let (pending, mut notifications) = watch::channel(Revision::default());
    let notifier = Notifier {
        pending: pending.clone(),
        following: Arc::default(),
    };
    let observed = paths.clone();
    let enqueue = move |event| {
        pending.send_if_modified(|revision| {
            let before = *revision;
            observed.read().unwrap().notice(event, revision);
            before != *revision
        });
    };
    let mut watcher = notify::recommended_watcher(enqueue.clone())?;
    // Recovery compares metadata rather than making every viewer render again
    // each minute. Its scan runs on notify's thread and never reads contents.
    let mut recovery =
        notify::PollWatcher::new(enqueue, notify::Config::default().with_manual_polling())?;
    let mut watched = BTreeMap::new();
    let mut recovered = BTreeMap::new();
    reconcile(&mut watcher, &mut watched, &paths.read().unwrap());
    reconcile(&mut recovery, &mut recovered, &paths.read().unwrap());
    let (sender, changes) = watch::channel(Revision::default());
    let observer = tokio::spawn(async move {
        let mut previous = Revision::default();
        let mut recovery_tick = tokio::time::interval(Duration::from_secs(60));
        recovery_tick.tick().await;
        loop {
            let fallback = tokio::select! {
                _ = recovery_tick.tick() => true,
                result = notifications.changed() => {
                    if result.is_err() { return; }
                    false
                }
            };
            // Coalesce an atomic save and bursts of log appends into one notice.
            if !fallback {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if fallback && let Err(error) = recovery.poll() {
                eprintln!("thirdshift Console: filesystem recovery: {error}");
            }
            let revision = *notifications.borrow_and_update();
            if (fallback || revision.accounts != previous.accounts)
                && let Ok(config) = crate::config::UserConfig::load()
            {
                paths.write().unwrap().logs = observed_path(&config.logs_dir);
            }
            reconcile(&mut watcher, &mut watched, &paths.read().unwrap());
            reconcile(&mut recovery, &mut recovered, &paths.read().unwrap());
            sender.send_if_modified(|current| {
                let before = *current;
                if revision.commands != previous.commands {
                    current.commands += 1;
                }
                // Session expiry follows wall time even when no file changes.
                if fallback || revision.accounts != previous.accounts {
                    current.accounts += 1;
                }
                if revision.executable != previous.executable {
                    current.executable += 1;
                }
                if revision.styles != previous.styles {
                    current.styles += 1;
                }
                if revision.scripts != previous.scripts {
                    current.scripts += 1;
                }
                *current != before
            });
            previous = revision;
        }
    });
    Ok((changes, notifier, observer))
}

fn reconcile(
    watcher: &mut impl Watcher,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_backend_events_match_paths_under_a_symlinked_home() {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let root = alias.join(".thirdshift");
        std::fs::create_dir(&root).unwrap();
        // The log root need not exist yet. FSEvents reports canonical paths,
        // including /private/var for a HOME supplied under macOS's /var alias.
        let paths = Paths::new(root.clone(), root.join("logs"), alias.join("thirdshift"));
        let canonical = real.canonicalize().unwrap().join(".thirdshift");
        let mut revision = Revision::default();
        for file in [
            "config.toml",
            "console-accounts.json",
            "logs/new.command.json",
        ] {
            paths.notice(
                Ok(Event::new(EventKind::Any).add_path(canonical.join(file))),
                &mut revision,
            );
        }
        assert_eq!(revision.accounts, 2);
        assert_eq!(revision.commands, 2);
    }

    #[test]
    fn directory_metadata_alone_does_not_invalidate_the_inbox() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(".thirdshift");
        std::fs::create_dir(&root).unwrap();
        let paths = Paths::new(
            root.clone(),
            root.join("logs"),
            temp.path().join("thirdshift"),
        );
        let mut revision = Revision::default();
        for path in [
            temp.path().canonicalize().unwrap(),
            root.canonicalize().unwrap(),
        ] {
            paths.notice(
                Ok(
                    Event::new(EventKind::Modify(notify::event::ModifyKind::Metadata(
                        notify::event::MetadataKind::WriteTime,
                    )))
                    .add_path(path),
                ),
                &mut revision,
            );
        }
        assert!(revision == Revision::default());
    }
}
