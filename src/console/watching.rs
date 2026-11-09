//! Native process exits, referenced logs outside logs.dir, and health deadlines.
//! All viewers share this observer; the last viewer dropping stops it.
use std::collections::BTreeSet;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::Duration;

use anyhow::Result;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tokio::io::unix::AsyncFd;
use tokio::task::{JoinHandle, JoinSet};

use super::changes::{Notifier, Receiver};
use crate::config::UserConfig;
use crate::logs::status::{self, Process};

pub(super) struct Following(JoinHandle<()>);

impl Drop for Following {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
impl Following {
    pub(super) fn idle() -> Self {
        Self(tokio::spawn(std::future::pending()))
    }
}

pub(super) fn follow(notifier: Notifier, mut changes: Receiver) -> Following {
    Following(tokio::spawn(async move {
        let mut initial = true;
        loop {
            let seen = changes.borrow_and_update().commands;
            let result = tokio::task::spawn_blocking(|| -> Result<_> {
                let config = UserConfig::load()?;
                let watching = status::watching(&config)?;
                Ok((config, watching))
            })
            .await;
            let (config, watching) = match result {
                Ok(Ok(watching)) => watching,
                error => {
                    eprintln!("thirdshift Console: can't follow live work: {error:?}");
                    if changes.changed().await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            let paths: BTreeSet<_> = watching
                .paths
                .iter()
                .map(|path| super::changes::observed_path(path))
                .collect();
            let logs = super::changes::observed_path(&config.logs_dir);
            let changed = notifier.clone();
            let filter = paths.clone();
            let mut files =
                notify::recommended_watcher(move |result: notify::Result<Event>| match result {
                    Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
                    Ok(event) if !event.need_rescan() => {
                        if event.paths.iter().any(|path| {
                            filter
                                .iter()
                                .any(|file| file == path || file.starts_with(path))
                        }) {
                            changed.commands();
                        }
                    }
                    _ => changed.commands(),
                })
                .ok();
            if let Some(files) = &mut files {
                let parents: BTreeSet<_> = paths
                    .iter()
                    .filter(|path| !path.starts_with(&logs))
                    .filter_map(|path| path.parent()?.ancestors().find(|parent| parent.is_dir()))
                    .collect();
                for parent in parents {
                    if let Err(error) = files.watch(parent, RecursiveMode::NonRecursive) {
                        eprintln!(
                            "thirdshift Console: can't watch {}: {error}",
                            parent.display()
                        );
                    }
                }
            }
            let mut processes = JoinSet::new();
            let mut pids = BTreeSet::new();
            for process in watching.processes {
                if pids.insert(process.pid) {
                    processes.spawn(exited(process));
                }
            }
            // Catch a change racing the first dependency scan, even if the
            // process had already died when that scan observed it.
            if initial {
                initial = false;
                notifier.commands();
            }
            let deadline = async {
                match watching.deadline {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    _ = processes.join_next(), if !processes.is_empty() => {
                        notifier.commands();
                        break;
                    }
                    _ = &mut deadline => {
                        notifier.commands();
                        break;
                    }
                    result = changes.changed() => {
                        if result.is_err() { return; }
                        if changes.borrow_and_update().commands != seen { break; }
                    }
                }
            }
            // Dropping JoinSet cancels every process wait and closes its fd.
        }
    }))
}

async fn exited(mut process: Process) {
    let prepared = tokio::task::spawn_blocking(move || {
        let descriptor = exit_descriptor(process.pid);
        let live = process.live();
        (process, descriptor, live)
    })
    .await;
    let Ok((selected, descriptor, live)) = prepared else {
        return;
    };
    if !live {
        return;
    }
    process = selected;
    if let Ok(descriptor) = descriptor
        && let Ok(descriptor) = AsyncFd::new(descriptor)
        && descriptor.readable().await.is_ok()
    {
        return;
    }
    // Older kernels or restricted process notifications: sample only this
    // process identity, never the store or accounts, until it exits.
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let checked = tokio::task::spawn_blocking(move || (process.live(), process)).await;
        let Ok((live, selected)) = checked else {
            return;
        };
        if !live {
            return;
        }
        process = selected;
    }
}

fn exit_descriptor(pid: u32) -> std::io::Result<OwnedFd> {
    // SAFETY: pidfd_open returns a new close-on-exec descriptor owned here.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}
