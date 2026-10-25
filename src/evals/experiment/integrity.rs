//! Protect the inputs and commits under review while allowing session work.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::super::{format::Task, repository};
use crate::prompt;

pub(super) struct Snapshot {
    base: String,
    change: String,
    inputs: BTreeSet<String>,
    digest: Vec<u8>,
}

impl Snapshot {
    pub fn capture(repo: &Path, task: &Task) -> Result<Self> {
        let inputs = task.trial.as_ref().context("missing Trial inputs")?;
        let mut snapshot = Self {
            base: format!("refs/heads/{}", inputs.base),
            change: repository::git(repo, &["rev-parse", "HEAD"])?,
            inputs: task.inputs.keys().cloned().collect(),
            digest: Vec::new(),
        };
        snapshot.inputs.insert(prompt::ISSUE_FILE.into());
        snapshot.digest = snapshot.digest(repo)?;
        Ok(snapshot)
    }

    pub fn unchanged(&self, repo: &Path) -> Result<bool> {
        Ok(self.digest(repo)? == self.digest)
    }

    fn digest(&self, repo: &Path) -> Result<Vec<u8>> {
        let mut digest = Sha256::new();
        hash(
            &mut digest,
            repository::git(repo, &["rev-parse", &self.base])?.as_bytes(),
        );
        // Pin the prepared change object, not the Issue branch's moving HEAD.
        // Include both commits' trees and blobs, even binary or ignored paths,
        // so corrupting an object cannot hide behind its unchanged object ID.
        let objects = repository::git(
            repo,
            &[
                "rev-list",
                "--objects",
                "--no-object-names",
                "--no-walk",
                &self.base,
                &self.change,
            ],
        )?;
        hash(
            &mut digest,
            &repository::bytes(
                repo,
                &["cat-file", "--batch"],
                Some(format!("{objects}\n").as_bytes()),
            )?,
        );
        let objects: BTreeSet<_> = objects.lines().collect();
        // Replacement refs for these objects change what the review sees.
        // Other new refs, including local-origin pushes, remain ordinary work.
        let replacements = repository::git(
            repo,
            &[
                "for-each-ref",
                "--format=%(refname:strip=2) %(objectname)",
                "refs/replace/",
            ],
        )?;
        for replacement in replacements.lines() {
            if replacement
                .split_once(' ')
                .is_some_and(|(id, _)| objects.contains(id))
            {
                hash(&mut digest, replacement.as_bytes());
            }
        }
        for name in &self.inputs {
            hash_input(&mut digest, repo, name)?;
        }
        Ok(digest.finalize().to_vec())
    }
}

fn hash_input(digest: &mut Sha256, repo: &Path, name: &str) -> Result<()> {
    let path = repo.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            hash(digest, name.as_bytes());
            hash(digest, &metadata.permissions().mode().to_le_bytes());
            if metadata.is_symlink() {
                hash(digest, fs::read_link(path)?.as_os_str().as_encoded_bytes());
            } else if metadata.is_file() {
                hash(digest, &fs::read(path)?);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hash(digest, name.as_bytes());
            hash(digest, b"missing");
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn hash(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}
