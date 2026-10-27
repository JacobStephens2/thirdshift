//! The Log inventory: one capture of the retained Command logs and Session
//! traces beneath the log folder. Command reading, historical recovery,
//! Session queries and the Logs listing share a capture, so repeated queries
//! reuse its enumeration, filename decoding, Model evidence and modification
//! times. The next request captures afresh; Readers check eligibility alone.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};

use crate::logs::owner::{self, SessionOwner};

/// The filesystem effects of discovery, separable so tests can count them.
pub(super) trait Files: Send + Sync {
    /// A directory's entries, each with its own type: symlinks are `Other`.
    fn list(&self, path: &Path) -> io::Result<Vec<(PathBuf, EntryKind)>>;
    fn metadata(&self, path: &Path) -> io::Result<std::fs::Metadata>;
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryKind {
    File,
    Directory,
    Other,
}

pub(super) struct Native;

impl Files for Native {
    fn list(&self, path: &Path) -> io::Result<Vec<(PathBuf, EntryKind)>> {
        std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                let kind = entry.file_type()?;
                let kind = if kind.is_file() {
                    EntryKind::File
                } else if kind.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::Other
                };
                Ok((entry.path(), kind))
            })
            .collect()
    }

    fn metadata(&self, path: &Path) -> io::Result<std::fs::Metadata> {
        std::fs::metadata(path)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
}

pub(super) struct Inventory {
    files: Arc<dyn Files>,
    repositories: Vec<Repository>,
    /// Kind folders by lexical path, plus any folder a Session query named
    /// outside the listed layout, each enumerated once.
    folders: Mutex<HashMap<PathBuf, Arc<Folder>>>,
    /// Session ordering: best effort, with UNIX_EPOCH when unavailable.
    modified: Mutex<HashMap<PathBuf, SystemTime>>,
    /// Activity is sampled on first use, including unavailable metadata.
    activity: Mutex<HashMap<PathBuf, FileActivity>>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct FileActivity {
    pub(super) length: u64,
    pub(super) modified: Option<SystemTime>,
}

/// An `owner/repository` folder and its real `commands/<kind>` folders.
pub(super) struct Repository {
    pub(super) path: PathBuf,
    pub(super) folders: Vec<Arc<Folder>>,
}

impl Repository {
    /// `owner/repository`, from the folder names as spelled.
    pub(super) fn name(&self) -> String {
        name(&self.path)
    }
}

pub(super) fn name(repo: &Path) -> String {
    format!(
        "{}/{}",
        repo.parent()
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy(),
        repo.file_name().unwrap_or_default().to_string_lossy()
    )
}

pub(super) struct Folder {
    pub(super) path: PathBuf,
    /// Regular files only, never symlinks, in enumeration order.
    pub(super) files: Vec<PathBuf>,
    sessions: OnceLock<Sessions>,
}

/// Inferred Session candidates, indexed by subject and effective owner.
struct Sessions {
    candidates: Vec<Candidate>,
    index: HashMap<(String, SessionOwner), Vec<usize>>,
}

/// A Session trace's filename facts and optional Model evidence, decoded once.
pub(super) struct Candidate {
    pub(super) path: PathBuf,
    pub(super) subject: String,
    pub(super) time: DateTime<FixedOffset>,
    owner: SessionOwner,
    named_log: PathBuf,
}

impl Candidate {
    /// The evidenced Command log, otherwise the one its filename names.
    pub(super) fn command_log(&self) -> &Path {
        match &self.owner {
            SessionOwner::Evidenced(log) => log,
            SessionOwner::Named { .. } => &self.named_log,
        }
    }

    pub(super) fn owned_by(&self, log: &Path) -> bool {
        self.owner.owns(log)
    }
}

impl Inventory {
    fn new(files: Arc<dyn Files>) -> Self {
        Self {
            files,
            repositories: Vec::new(),
            folders: Mutex::new(HashMap::new()),
            modified: Mutex::new(HashMap::new()),
            activity: Mutex::new(HashMap::new()),
        }
    }

    /// Only the root and the literal `commands` folder may be aliases; owner,
    /// repository and kind entries must be real directories.
    pub(super) fn capture(files: Arc<dyn Files>, root: &Path) -> Result<Self> {
        let mut inventory = Self::new(files);
        for owner in inventory.directories(root)? {
            for path in inventory.directories(&owner)? {
                let mut folders = Vec::new();
                for folder in inventory.directories(&path.join("commands"))? {
                    folders.push(inventory.folder(&folder)?);
                }
                inventory.repositories.push(Repository { path, folders });
            }
        }
        Ok(inventory)
    }

    /// A capture holding just the folders its queries name, such as one
    /// Command's folder reloaded after Stop.
    pub(super) fn on_demand() -> Self {
        Self::new(Arc::new(Native))
    }

    pub(super) fn repositories(&self) -> &[Repository] {
        &self.repositories
    }

    fn list(&self, path: &Path) -> Result<Vec<(PathBuf, EntryKind)>> {
        match self.files.list(path) {
            Ok(entries) => Ok(entries),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error).with_context(|| format!("could not list {}", path.display())),
        }
    }

    pub(super) fn directories(&self, path: &Path) -> Result<Vec<PathBuf>> {
        Ok(self
            .list(path)?
            .into_iter()
            .filter(|(_, kind)| *kind == EntryKind::Directory)
            .map(|(path, _)| path)
            .collect())
    }

    /// The captured folder at this lexical path, enumerating it on first use.
    pub(super) fn folder(&self, path: &Path) -> Result<Arc<Folder>> {
        if let Some(folder) = self.folders.lock().unwrap().get(path) {
            return Ok(folder.clone());
        }
        let folder = Arc::new(Folder {
            path: path.to_owned(),
            files: self
                .list(path)?
                .into_iter()
                .filter(|(_, kind)| *kind == EntryKind::File)
                .map(|(path, _)| path)
                .collect(),
            sessions: OnceLock::new(),
        });
        Ok(self
            .folders
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_insert(folder)
            .clone())
    }

    fn sessions<'f>(&self, folder: &'f Folder) -> &'f Sessions {
        folder.sessions.get_or_init(|| {
            let mut sessions = Sessions {
                candidates: Vec::new(),
                index: HashMap::new(),
            };
            for path in &folder.files {
                if !owner::inventory_session(path) {
                    continue;
                }
                let Some(decoded) =
                    owner::decode(path).filter(|decoded| decoded.session_kind.is_some())
                else {
                    continue;
                };
                // Unreadable or absent evidence falls back to the filename.
                let evidence = self
                    .files
                    .read(&owner::evidence_file(path))
                    .ok()
                    .and_then(|sidecar| owner::evidenced_log(&sidecar));
                let candidate = Candidate {
                    path: path.clone(),
                    subject: decoded.subject.into(),
                    time: decoded.time(),
                    named_log: decoded.command_log(path),
                    owner: SessionOwner::of(&decoded, evidence),
                };
                sessions
                    .index
                    .entry((candidate.subject.clone(), candidate.owner.clone()))
                    .or_default()
                    .push(sessions.candidates.len());
                // Catalogue reviews retain their Security review kind, with
                // the Process Step in the subject to keep every trace. Also
                // discover them through their Process's usual subject, with
                // exactly the same owner constraint as every other Session.
                if candidate
                    .subject
                    .strip_prefix("process-step-")
                    .and_then(|step| step.parse::<usize>().ok())
                    .is_some_and(|step| step > 0)
                {
                    sessions
                        .index
                        .entry(("process".to_string(), candidate.owner.clone()))
                        .or_default()
                        .push(sessions.candidates.len());
                }
                sessions.candidates.push(candidate);
            }
            sessions
        })
    }

    /// Every Session candidate in a captured folder, in enumeration order.
    pub(super) fn candidates<'f>(&self, folder: &'f Folder) -> &'f [Candidate] {
        &self.sessions(folder).candidates
    }

    /// Discovered Sessions beside `log` that name `subject` and belong to it.
    pub(super) fn sessions_of(&self, log: &Path, subject: &str) -> Result<Vec<PathBuf>> {
        let folder = self.folder(log.parent().context("Command log has no directory")?)?;
        let sessions = self.sessions(&folder);
        Ok(SessionOwner::of_log(log)
            .filter_map(|owner| sessions.index.get(&(subject.to_owned(), owner)))
            .flatten()
            .map(|&index| sessions.candidates[index].path.clone())
            .collect())
    }

    /// Order Sessions by modification time, then exact path, ascending.
    pub(super) fn ordered(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut modified = self.modified.lock().unwrap();
        let mut dated: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let time = *modified.entry(path.clone()).or_insert_with(|| {
                    self.files
                        .metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH)
                });
                (time, path)
            })
            .collect();
        dated.sort();
        dated.into_iter().map(|(_, path)| path).collect()
    }

    pub(super) fn modified(&self, path: &Path) -> Result<Option<SystemTime>> {
        Ok(self.files.metadata(path)?.modified().ok())
    }

    pub(super) fn activity(&self, path: &Path) -> FileActivity {
        *self
            .activity
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_insert_with(|| {
                self.files.metadata(path).map_or_else(
                    |_| FileActivity::default(),
                    |file| FileActivity {
                        length: file.len(),
                        modified: file.modified().ok(),
                    },
                )
            })
    }

    pub(super) fn is_file(&self, path: &Path) -> bool {
        self.files
            .metadata(path)
            .is_ok_and(|metadata| metadata.is_file())
    }
}
