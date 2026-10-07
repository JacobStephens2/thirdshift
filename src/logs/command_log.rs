//! Stateless Command log file effects. The recording owner decides its
//! path, held lines, lifetime and warning policy.

use std::fs::{self, File};
use std::path::Path;

use anyhow::{Context, Result};

pub(super) fn open(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    File::create(path).with_context(|| format!("could not create {}", path.display()))
}
