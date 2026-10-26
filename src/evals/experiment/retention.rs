//! Discard only disposable Trial state, after its last possible reader.
//! Cleanup is best effort and never participates in the Experiment's result.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};

use super::Conclusion;
use super::store::{State, Stored};

/// The outer runner owns cleanup, including for old Arms and protocol errors.
pub(super) struct Caches(pub PathBuf);

impl Drop for Caches {
    fn drop(&mut self) {
        for relative in ["session/home/.codex/cache", "session/home/.cache"] {
            let path = self.0.join(relative);
            warn(&path, || {
                let mut parent = self.0.clone();
                for component in Path::new(relative).parent().unwrap().components() {
                    parent.push(component);
                    let metadata = match fs::symlink_metadata(&parent) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                        Err(error) => return Err(error.into()),
                    };
                    ensure!(
                        metadata.is_dir(),
                        "cache ancestor is not a directory: {}",
                        parent.display()
                    );
                }
                remove_directory(&path)
            });
        }
    }
}

pub(super) fn repositories(experiment: &Stored) {
    if experiment.state != State::Completed
        || !experiment.pending_rechecks.is_empty()
        || experiment
            .verdict
            .as_ref()
            .is_none_or(|verdict| matches!(verdict.verdict, Conclusion::WaitingOnRechecks))
    {
        return;
    }
    let trials = experiment.directory.join("trials");
    warn(&trials, || {
        for trial in fs::read_dir(&trials)? {
            let trial = trial?;
            if !trial.file_type()?.is_dir() {
                continue;
            }
            let path = trial.path();
            warn(&path, || {
                for attempt in fs::read_dir(&path)? {
                    let attempt = attempt?;
                    if attempt.file_type()?.is_dir()
                        && attempt
                            .file_name()
                            .to_string_lossy()
                            .starts_with("attempt-")
                    {
                        remove(&attempt.path().join("repository"));
                    }
                }
                Ok(())
            });
        }
        Ok(())
    });
}

fn remove(path: &Path) {
    warn(path, || remove_directory(path));
}

fn remove_directory(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => Ok(result?),
    }
}

fn warn(path: &Path, operation: impl FnOnce() -> Result<()>) {
    if let Err(error) = operation() {
        crate::progress::step(format!(
            "warning: could not prune {}: {error:#}",
            path.display()
        ));
    }
}
