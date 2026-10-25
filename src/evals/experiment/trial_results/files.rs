//! Private filesystem seam shared with retained Session evidence discovery.

#[cfg(test)]
pub(in crate::evals::experiment) mod observed;

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;

pub(in crate::evals::experiment) trait Files {
    fn entries(&self, path: &Path) -> io::Result<Vec<Entry>>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn open(&self, path: &Path) -> io::Result<File>;
}

pub(in crate::evals::experiment) struct Entry {
    pub path: PathBuf,
    pub kind: fs::FileType,
}

pub(in crate::evals::experiment) struct Native;

impl Files for Native {
    fn entries(&self, path: &Path) -> io::Result<Vec<Entry>> {
        fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok(Entry {
                    path: entry.path(),
                    kind: entry.file_type()?,
                })
            })
            .collect()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }

    fn open(&self, path: &Path) -> io::Result<File> {
        File::open(path)
    }
}

pub(in crate::evals::experiment) fn optional<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(in crate::evals::experiment) fn metadata<T: DeserializeOwned>(
    files: &impl Files,
    path: &Path,
) -> Result<Option<T>> {
    optional(files.read(path))?
        .map(|bytes| {
            serde_json::from_slice(&bytes).with_context(|| format!("can't read {}", path.display()))
        })
        .transpose()
}

pub(in crate::evals::experiment) fn directory(
    files: &impl Files,
    path: &Path,
) -> Result<Vec<Entry>> {
    let mut entries = optional(files.entries(path))?.unwrap_or_default();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}
