//! A Command holds a shared host lock from admission through exit. Experiments
//! probe it while draining work, covering admitted Commands before their records
//! are published. It imposes no limit and never refuses a hand-started Command.
use std::fs::{File, TryLockError};

use anyhow::{Context, Result};

fn open() -> Result<File> {
    let root = crate::config::home()?.join(".thirdshift");
    std::fs::create_dir_all(&root)?;
    let path = root.join("experiment-work.lock");
    File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("can't open {}", path.display()))
}

pub(crate) fn hold_command() -> Result<File> {
    let lock = open()?;
    lock.lock_shared()?;
    Ok(lock)
}

pub(crate) fn busy() -> Result<bool> {
    let lock = open()?;
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(TryLockError::WouldBlock) => Ok(true),
        Err(TryLockError::Error(error)) => Err(error.into()),
    }
}
