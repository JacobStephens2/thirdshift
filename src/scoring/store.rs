//! Retained Scoring samples keep their exact file identity across updates.
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::Week;
use crate::{config, interrupt};

pub(super) struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open() -> Result<Self> {
        Ok(Self::new(config::home()?.join(".thirdshift/evals/scoring")))
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn load(&self) -> Result<Vec<Retained>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut samples = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let week = serde_json::from_slice(&fs::read(&path)?)
                    .with_context(|| format!("can't read Scoring sample {}", path.display()))?;
                samples.push(Retained { week, path });
            }
        }
        samples.sort_by_key(|sample| sample.week.window_end);
        Ok(samples)
    }

    pub fn stage_packets(&self) -> Result<PacketStaging> {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)?;
        let directory = tempfile::Builder::new()
            .prefix("packets-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(&self.root)?;
        Ok(PacketStaging { directory })
    }
}

/// Payload access never exposes the checkpoint name. Writers hold the existing
/// machine-wide Scoring lock across their operation, including each commit.
pub(super) struct Retained {
    pub week: Week,
    path: PathBuf,
}

impl Retained {
    pub fn into_week(self) -> Week {
        self.week
    }

    pub fn commit(&self) -> Result<()> {
        Ok(config::replace(
            &self.path,
            &serde_json::to_string_pretty(&self.week)?,
        )?)
    }

    pub fn packet(&self, index: usize) -> Result<PathBuf> {
        let packets = self
            .week
            .packets
            .as_ref()
            .context("unscored sample has no retained packets")?;
        Ok(self
            .path
            .parent()
            .context("Scoring sample has no parent")?
            .join(packets)
            .join(index.to_string()))
    }
}

/// Dropping preparation or failing publication removes its private packets.
/// Only successful no-clobber sample publication transfers their ownership.
pub(super) struct PacketStaging {
    directory: tempfile::TempDir,
}

impl PacketStaging {
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    pub fn publish(self, mut week: Week) -> Result<Retained> {
        let root = self
            .path()
            .parent()
            .context("packet directory has no parent")?;
        week.packets = Some(
            self.path()
                .file_name()
                .context("packet directory has no name")?
                .to_string_lossy()
                .into_owned(),
        );
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        serde_json::to_writer_pretty(&mut file, &week)?;
        file.flush()?;
        file.as_file().sync_all()?;
        interrupt::check()?;
        let stamp = week.window_end.format("%Y%m%dT%H%M%SZ");
        let path = root.join(format!("{stamp}.json"));
        // Same-second runs keep distinct samples; neither name may clobber.
        let path = if path.exists() {
            root.join(format!("{stamp}-{}.json", week.seed))
        } else {
            path
        };
        file.persist_noclobber(&path)?;
        let _ = self.directory.keep();
        Ok(Retained { week, path })
    }
}

#[cfg(test)]
mod tests;
