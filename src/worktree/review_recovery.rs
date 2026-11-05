//! Durable authority for disposable scratch and retained pull-request checkouts.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use super::{Checkout, Directory};
use crate::git::Git;
use crate::{interrupt, progress, skills};

const TOKEN: &str = ".thirdshift-review-token";
const RECORD: &str = "thirdshift-review.json";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    nonce: String,
    detached: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    captured_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch_existed: Option<bool>,
    root: DirectoryRecord,
    admin: DirectoryRecord,
    common: DirectoryRecord,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryRecord {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl DirectoryRecord {
    fn capture(directory: &Directory) -> Result<Self> {
        directory.verify()?;
        let path = directory.path.canonicalize()?;
        if path != directory.path {
            bail!(
                "directory path is not canonical: {}",
                directory.path.display()
            );
        }
        let metadata = directory.handle.metadata()?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn verify(&self, directory: &Directory) -> Result<()> {
        let current = Self::capture(directory)?;
        if self.path != current.path || self.device != current.device || self.inode != current.inode
        {
            bail!("recorded directory {} was replaced", self.path.display());
        }
        Ok(())
    }
}

/// Keep handles for attempt-owned artifacts. Automatic temporary-file unlink
/// is disabled: a replacement at its pathname must never be deleted on error.
#[derive(Default)]
pub(in crate::worktree) struct Publication {
    token: Option<Artifact>,
    record: Option<NamedTempFile>,
}

impl Publication {
    pub(in crate::worktree) fn publish(
        &mut self,
        launch: &Git,
        checkout: &Checkout,
        start: &str,
        branch_existed: bool,
    ) -> Result<()> {
        let git = Git::new(checkout.path());
        if git.succeeds(&["ls-files", "--error-unmatch", "--", TOKEN])? {
            bail!("review token path is tracked; refusing to replace it");
        }
        skills::exclude(&git, &format!("/{TOKEN}"))?;
        if !git.succeeds(&["check-ignore", "--quiet", "--", TOKEN])? {
            bail!("review token is not excluded by Git; check repository ignore rules");
        }
        let mut temporary = tempfile::Builder::new()
            .prefix(".thirdshift-review-")
            .rand_bytes(32)
            .tempfile_in(&checkout.admin.path)?;
        temporary.disable_cleanup(true);
        self.record = Some(temporary);
        let temporary = self
            .record
            .as_mut()
            .context("missing prepared review record")?;
        // The exclusively created temporary filename supplies a new nonce.
        let nonce = temporary
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .context("review nonce is not UTF-8")?
            .to_string();
        let token_path = checkout.path().join(TOKEN);
        let token_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&token_path)
            .context("cannot exclusively create review token")?;
        self.token = Some(Artifact {
            path: token_path,
            file: token_file,
        });
        let token = self.token.as_mut().context("missing review token")?;
        token.file.write_all(nonce.as_bytes())?;
        token.file.sync_all()?;
        let record = Record {
            version: VERSION,
            nonce,
            detached: checkout.branch.is_none(),
            branch: checkout.branch.clone(),
            captured_head: checkout.branch.as_ref().map(|_| start.to_string()),
            branch_existed: checkout.branch.as_ref().map(|_| branch_existed),
            root: DirectoryRecord::capture(&checkout.root)?,
            admin: DirectoryRecord::capture(&checkout.admin)?,
            common: DirectoryRecord::capture(&checkout.common)?,
        };
        temporary.write_all(&serde_json::to_vec(&record)?)?;
        temporary.as_file().sync_all()?;
        checkout.inspect(launch)?;
        token.verify()?;
        verify_file(temporary.path(), temporary.as_file())?;
        interrupt::check()?;
        // Publication commits acquisition. persist_noclobber has no fallible
        // step after publishing the destination on supported Linux/macOS.
        let temporary = self
            .record
            .take()
            .context("missing prepared review record")?;
        match temporary.persist_noclobber(checkout.admin.path.join(RECORD)) {
            Ok(_) => Ok(()),
            Err(error) => {
                self.record = Some(error.file);
                Err(error.error).context("cannot publish successful review acquisition")
            }
        }
    }

    pub(in crate::worktree) fn remove_artifacts(&self) -> Result<()> {
        if let Some(temporary) = &self.record {
            verify_file(temporary.path(), temporary.as_file())
                .context("prepared review record has uncertain ownership")?;
            fs::remove_file(temporary.path())
                .context("cannot remove attempt-owned review record")?;
        }
        if let Some(token) = &self.token {
            token
                .verify()
                .context("review token has uncertain ownership")?;
            fs::remove_file(&token.path).context("cannot remove attempt-owned review token")?;
        }
        Ok(())
    }
}

struct Artifact {
    path: PathBuf,
    file: File,
}

impl Artifact {
    fn read(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .with_context(|| format!("cannot read disposal evidence {}", path.display()))?;
        let artifact = Self { path, file };
        artifact.verify()?;
        Ok(artifact)
    }

    fn verify(&self) -> Result<()> {
        verify_file(&self.path, &self.file)
    }

    fn contents(&self) -> Result<Vec<u8>> {
        self.verify()?;
        let mut file = &self.file;
        file.rewind()?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        self.verify()?;
        Ok(contents)
    }
}

fn verify_file(path: &Path, file: &File) -> Result<()> {
    let current = fs::symlink_metadata(path)?;
    let pinned = file.metadata()?;
    if !current.is_file()
        || !pinned.is_file()
        || current.dev() != pinned.dev()
        || current.ino() != pinned.ino()
    {
        bail!(
            "evidence {} was replaced or is not a regular file",
            path.display()
        );
    }
    Ok(())
}

/// Pinned evidence is rechecked around every recovery mutation.
struct Evidence {
    record_file: Artifact,
    record_contents: Vec<u8>,
    token: Artifact,
    token_contents: Vec<u8>,
    record: Record,
}

impl Evidence {
    fn read(checkout: &Checkout) -> Result<Self> {
        let record_file = Artifact::read(checkout.admin.path.join(RECORD))?;
        let record_contents = record_file.contents()?;
        let record: Record = serde_json::from_slice(&record_contents)
            .context("invalid successful review acquisition record")?;
        if record.version != VERSION
            || record.nonce.is_empty()
            || record.detached != checkout.branch.is_none()
            || record.branch != checkout.branch
        {
            bail!("unsupported or inconsistent successful review acquisition record");
        }
        record.root.verify(&checkout.root)?;
        record.admin.verify(&checkout.admin)?;
        record.common.verify(&checkout.common)?;
        let token = Artifact::read(checkout.path().join(TOKEN))?;
        let token_contents = token.contents()?;
        if token_contents != record.nonce.as_bytes() {
            bail!("checkout token does not match successful review acquisition record");
        }
        Ok(Self {
            record_file,
            record_contents,
            token,
            token_contents,
            record,
        })
    }

    fn verify(&self, checkout: &Checkout) -> Result<()> {
        self.record.root.verify(&checkout.root)?;
        self.record.admin.verify(&checkout.admin)?;
        self.record.common.verify(&checkout.common)?;
        if self.token.contents()? != self.token_contents
            || self.record_file.contents()? != self.record_contents
        {
            bail!("successful review disposal evidence changed; scratch retained");
        }
        Ok(())
    }
}

/// Both acquisition locks are held. This observation grants adoption, never disposal.
pub(in crate::worktree) fn retained_pull_request(
    launch: &Git,
    path: &Path,
    branch: &str,
    start: &str,
) -> Result<Checkout> {
    let mut checkout = Checkout::capture(launch, path, Some(branch))?;
    let evidence = Evidence::read(&checkout)?;
    if evidence.record.captured_head.as_deref() != Some(start) {
        bail!("retained Review worktree captured a different pull request head");
    }
    let branch_existed = evidence
        .record
        .branch_existed
        .context("missing Review branch ownership")?;
    verify_retained(launch, &checkout, &evidence, start)?;
    if branch_existed {
        checkout.preserve_local_branch();
    }
    Ok(checkout)
}

fn verify_retained(
    launch: &Git,
    checkout: &Checkout,
    evidence: &Evidence,
    start: &str,
) -> Result<()> {
    checkout.inspect(launch)?;
    evidence.verify(checkout)?;
    let git = Git::new(checkout.path());
    for name in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        match fs::symlink_metadata(checkout.admin.path.join(name)) {
            Ok(_) => bail!("retained Review worktree has an unfinished Git operation"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("cannot inspect retained Git operation"),
        }
    }
    if super::super::uncommitted_changes(
        |args| git.run(args),
        super::super::UntrackedFiles::DiscardValidationArtifacts,
    )? {
        bail!("retained Review worktree has uncommitted tracked changes");
    }
    if !git.succeeds(&["merge-base", "--is-ancestor", start, "HEAD"])? {
        bail!("retained Review worktree is not a descendant of the captured head");
    }
    evidence.verify(checkout)?;
    checkout.inspect(launch)?;
    Ok(())
}

pub(in crate::worktree) fn clean_retained(
    launch: &Git,
    checkout: &Checkout,
    start: &str,
) -> Result<()> {
    let evidence = Evidence::read(checkout)?;
    let git = Git::new(checkout.path());
    git.run_checked(&["clean", "-fdx", "-e", TOKEN, "-e", RECORD], || {
        verify_retained(launch, checkout, &evidence, start)
    })?;
    verify_retained(launch, checkout, &evidence, start)
}

/// The caller holds the worktree lock; reset requires the same clean evidence.
pub(in crate::worktree) fn restart_retained(
    launch: &Git,
    checkout: &Checkout,
    start: &str,
) -> Result<()> {
    let _refs_lock = launch.lock_worktree_refs()?;
    let evidence = Evidence::read(checkout)?;
    let git = Git::new(checkout.path());
    git.run_checked(&["reset", "--hard", start], || {
        verify_retained(launch, checkout, &evidence, start)
    })?;
    verify_retained(launch, checkout, &evidence, start)
}

/// The caller holds one worktree lock through inspection, disposal and add.
pub(in crate::worktree) fn recover(launch: &Git, path: &Path) -> Result<()> {
    let mut registered_head = None;
    let mut inspect_and_remove = || -> Result<()> {
        let entries = super::registrations(launch)?;
        registered_head = entries
            .iter()
            .find(|entry| entry.path == path)
            .and_then(|entry| entry.head.clone());
        let exists = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    bail!("checkout path is a symlink");
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if !exists && !entries.iter().any(|entry| entry.path == path) {
            return Ok(());
        }
        // Capture pins the current directories, then the record must identify
        // those exact instances. HEAD may have advanced while detached.
        let checkout = Checkout::capture(launch, path, None)?;
        let evidence = Evidence::read(&checkout)?;
        interrupt::check()?;
        checkout.remove_locked(launch, || evidence.verify(&checkout))?;
        progress::step(format_args!(
            "removed the leftover worktree {}",
            path.display()
        ));
        Ok(())
    };
    inspect_and_remove().map_err(|error| {
        let head = Git::new(path)
            .run(&["rev-parse", "--verify", "HEAD"])
            .ok()
            .or(registered_head);
        let message = format!(
            "retaining checkout {} (expected detached HEAD) at {}: {error:#}",
            path.display(),
            head.as_deref().unwrap_or("unknown head")
        );
        error.context(message)
    })
}
