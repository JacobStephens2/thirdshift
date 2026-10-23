//! Compare repository state around the session, before Grader build artifacts.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::{git::Git, prompt};

pub(super) fn snapshot(repository: &Path) -> Result<Vec<u8>> {
    let git = Git::new(repository);
    let mut digest = Sha256::new();
    for args in [
        vec!["rev-parse", "HEAD"],
        vec!["for-each-ref", "--format=%(refname) %(objectname)"],
        vec!["ls-files", "--stage", "-z"],
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--binary",
            "HEAD",
            "--",
        ],
    ] {
        hash(&mut digest, git.run(&args)?.as_bytes());
    }
    let files = git.run(&[
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "-z",
    ])?;
    let mut files: BTreeSet<_> = files.split('\0').filter(|path| !path.is_empty()).collect();
    // Trial setup excludes the issue from Git, but changing this input still
    // changes the repository. Factory skill links and report scratch are the
    // only generated files excluded by ordinary session setup.
    files.insert(prompt::ISSUE_FILE);
    for name in files {
        hash(&mut digest, name.as_bytes());
        let path = repository.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                hash(&mut digest, &metadata.permissions().mode().to_le_bytes());
                if metadata.is_symlink() {
                    hash(
                        &mut digest,
                        fs::read_link(path)?.as_os_str().as_encoded_bytes(),
                    );
                } else if metadata.is_file() {
                    hash(&mut digest, &fs::read(path)?);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hash(&mut digest, b"missing");
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(digest.finalize().to_vec())
}

fn hash(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
}
