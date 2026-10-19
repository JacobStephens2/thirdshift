//! Watch the installed path, rather than /proc's possibly unlinked executable.
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

#[derive(Clone, PartialEq)]
struct Identity {
    modified: SystemTime,
    size: u64,
    #[cfg(unix)]
    inode: (u64, u64),
}

#[derive(Clone)]
pub(super) struct Executable {
    path: PathBuf,
    identity: Identity,
}

impl Executable {
    pub fn current() -> Result<Self> {
        let path = std::env::current_exe().context("can't locate the Console binary")?;
        let identity = identity(&path)?;
        Ok(Self { path, identity })
    }

    pub async fn replaced(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if identity(&self.path).is_ok_and(|current| current != self.identity) {
                return;
            }
        }
    }

    pub fn exec(&self) -> Result<()> {
        let mut command = std::process::Command::new(&self.path);
        command.args(std::env::args_os().skip(1));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Err(command.exec()).context("can't re-execute the updated Console")
        }
        #[cfg(not(unix))]
        {
            command
                .spawn()
                .context("can't restart the updated Console")?;
            Ok(())
        }
    }
}

fn identity(path: &std::path::Path) -> Result<Identity> {
    let metadata = std::fs::metadata(path)?;
    Ok(Identity {
        modified: metadata.modified()?,
        size: metadata.len(),
        #[cfg(unix)]
        inode: {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        },
    })
}
