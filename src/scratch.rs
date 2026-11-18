//! One private scratch directory for a Command and all of its child Runs.

use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

static CURRENT: Mutex<Option<Directory>> = Mutex::new(None);
const INHERITED: &str = "THIRDSHIFT_SCRATCH";

struct Directory {
    path: PathBuf,
    owned: bool,
    _lease: Option<File>,
}

#[derive(Serialize, Deserialize)]
struct Owner {
    host: Option<String>,
    process: u32,
    started: String,
    record: Option<PathBuf>,
    command: Vec<String>,
    launch: PathBuf,
}

#[derive(Default)]
pub(crate) struct Retention {
    pub keep: bool,
    pub completed: bool,
    pub keys: Vec<Work>,
}

#[derive(PartialEq, Eq)]
pub(crate) enum Work {
    Issue(String),
    PullRequest(String),
    Experiment(String),
}

impl Owner {
    fn retention(&self) -> Result<Retention> {
        self.record
            .as_deref()
            .map(crate::logs::scratch::read)
            .transpose()
            .map(Option::unwrap_or_default)
    }
    fn dead(&self) -> bool {
        crate::reservation::owner_is_dead(self.host.clone(), self.process, self.started.clone())
    }
}

/// The CLI keeps this guard until every Command finishing path has returned.
pub(crate) struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let current = CURRENT
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(directory) = current
            && directory.owned
        {
            let result = (|| -> Result<()> {
                let owner = read_owner(&directory.path)?;
                let retention = owner.retention()?;
                if !retention.keep {
                    fs::remove_dir_all(&directory.path)?;
                    if retention.completed {
                        clean(
                            directory.path.parent().context("scratch has no root")?,
                            |previous| {
                                let prior = previous.retention()?;
                                let same_work =
                                    prior.keys.iter().any(|key| retention.keys.contains(key))
                                        || (prior.keys.is_empty()
                                            && retention.keys.is_empty()
                                            && previous.command == owner.command
                                            && previous.launch == owner.launch);
                                Ok(prior.keep && same_work)
                            },
                            "completed continuation's retained",
                        )?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                crate::progress::warn(&error, format_args!("could not finish Command scratch"));
            }
        }
    }
}

pub(crate) fn begin(child: bool) -> Result<()> {
    let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if current.is_some() {
        return Ok(());
    }
    if child {
        let path = std::env::var_os(INHERITED).context("missing parent Command scratch")?;
        let path = PathBuf::from(path);
        anyhow::ensure!(
            path.join("tmp").is_dir(),
            "parent Command scratch is missing"
        );
        *current = Some(Directory {
            path,
            owned: false,
            _lease: None,
        });
        return Ok(());
    }
    let root = crate::config::home()?.join(".thirdshift/scratch");
    fs::create_dir_all(&root)?;
    anyhow::ensure!(
        owned_directory(&root),
        "scratch root is not an owned directory"
    );
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let directory = tempfile::Builder::new()
        .prefix("command-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(&root)?;
    let lease = File::create(directory.path().join("lease"))?;
    lease.try_lock()?;
    let owner = Owner {
        host: crate::host::name(),
        process: std::process::id(),
        started: chrono::Utc::now().to_rfc3339(),
        record: None,
        command: std::env::args().skip(1).collect(),
        launch: std::env::current_dir()?.canonicalize()?,
    };
    fs::write(
        directory.path().join("owner.json"),
        serde_json::to_vec(&owner)?,
    )?;
    fs::create_dir(directory.path().join("tmp"))?;
    *current = Some(Directory {
        path: directory.keep(),
        owned: true,
        _lease: Some(lease),
    });
    drop(current);
    clean(&root, |owner| Ok(!owner.retention()?.keep), "orphaned")?;
    clean_legacy(&std::env::temp_dir())?;
    Ok(())
}

/// Native Trials are subprocesses of an Experiment, and own no Command.
#[cfg(not(test))]
pub(crate) fn begin_trial() -> Result<()> {
    begin(std::env::var_os(INHERITED).is_some())
}

fn clean_legacy(root: &Path) -> Result<()> {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 86400);
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("thirdshift-skills-")
            || !owned_directory(&path)
            || !fs::symlink_metadata(&path)?
                .modified()
                .is_ok_and(|time| time < cutoff)
            || !legacy_layout(&path, &crate::skills::SKILLS)?
        {
            continue;
        }
        fs::remove_dir_all(&path)?;
        event(format!(
            "removed legacy Factory skills {} (older than 2 days)",
            path.display()
        ));
    }
    Ok(())
}

/// Known embedded paths only, recursively: no extra user files or symlinks.
/// Older extractions may contain fewer skills than the current binary.
fn legacy_layout(path: &Path, expected: &include_dir::Dir<'_>) -> Result<bool> {
    let mut saw_skill = false;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        // SAFETY: geteuid has no preconditions.
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Ok(false);
        }
        let relative = expected.path().join(entry.file_name());
        if metadata.is_dir() {
            let Some(directory) = expected.get_dir(&relative) else {
                return Ok(false);
            };
            if expected.path().as_os_str().is_empty() {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("thirdshift-")
                    || !entry.path().join("SKILL.md").is_file()
                {
                    return Ok(false);
                }
                saw_skill = true;
            }
            if !legacy_layout(&entry.path(), directory)? {
                return Ok(false);
            }
        } else if !metadata.is_file() || expected.get_file(&relative).is_none() {
            return Ok(false);
        }
    }
    Ok(!expected.path().as_os_str().is_empty() || (saw_skill && path.join("LICENSE").is_file()))
}

fn owned_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        // SAFETY: geteuid has no preconditions or retained pointers.
        metadata.is_dir() && metadata.uid() == unsafe { libc::geteuid() }
    })
}

fn clean(root: &Path, remove: impl Fn(&Owner) -> Result<bool>, reason: &str) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_name().to_string_lossy().starts_with("command-") || !owned_directory(&path) {
            continue;
        }
        let result = (|| -> Result<()> {
            let lease = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path.join("lease"))?;
            if lease.try_lock().is_err() {
                return Ok(());
            }
            let owner = read_owner(&path)?;
            if owner.dead() && remove(&owner)? {
                fs::remove_dir_all(&path)?;
                event(format!(
                    "removed {reason} Command scratch {} (process {}; started {})",
                    path.display(),
                    owner.process,
                    owner.started
                ));
            }
            Ok(())
        })();
        if let Err(error) = result {
            crate::progress::warn(
                &error,
                format_args!("could not inspect scratch {}", path.display()),
            );
        }
    }
    Ok(())
}

fn read_owner(path: &Path) -> Result<Owner> {
    Ok(serde_json::from_slice(&fs::read(path.join("owner.json"))?)?)
}

/// Called before publishing the owning record. Child records do not replace it.
pub(crate) fn record(record: &Path) {
    let current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(directory) = &*current else {
        return;
    };
    if !directory.owned {
        return;
    }
    let result = (|| -> Result<()> {
        let mut owner = read_owner(&directory.path)?;
        if owner.record.as_deref() != Some(record) {
            owner.record = Some(record.to_owned());
            let pending = directory.path.join("owner.pending");
            fs::write(&pending, serde_json::to_vec(&owner)?)?;
            fs::rename(pending, directory.path.join("owner.json"))?;
        }
        Ok(())
    })();
    // This runs while recording is locked; report without re-entering logs.
    if let Err(error) = result {
        eprintln!("thirdshift: could not attach scratch to its Command record: {error:#}");
    }
}

/// Dismissal is shared by the CLI and Console, and never removes live scratch.
pub(crate) fn dismiss(record: &Path) -> Result<()> {
    let root = crate::config::home()?.join(".thirdshift/scratch");
    if root.exists() {
        clean(
            &root,
            |owner| Ok(owner.record.as_deref() == Some(record)),
            "dismissed",
        )?;
    }
    Ok(())
}

fn event(message: String) {
    crate::logs::activity(&message);
    crate::logs::show_held();
    crate::progress::step(message);
}

/// Unit tests without a Command keep their existing TempDir ownership.
pub(crate) fn temp_dir() -> Option<PathBuf> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|directory| directory.path.join("tmp"))
}

/// Every production Session must have scratch even outside Command logging.
pub(crate) fn session_temp() -> Result<Option<PathBuf>> {
    #[cfg(not(test))]
    begin(false)?;
    Ok(temp_dir())
}

/// Bind only this sandbox's temporary files; other Trials' files stay hidden.
pub(crate) fn sandbox_temp() -> Result<Option<PathBuf>> {
    session_temp()?
        .map(|tmp| {
            tempfile::Builder::new()
                .prefix("session-")
                .permissions(fs::Permissions::from_mode(0o700))
                .tempdir_in(tmp)
                .map(|dir| dir.keep())
                .map_err(Into::into)
        })
        .transpose()
}

pub(crate) fn inherit(command: &mut std::process::Command) {
    let current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(directory) = &*current {
        command.env(INHERITED, &directory.path);
    } else {
        command.env_remove(INHERITED);
    }
}

pub(crate) fn skills_directory() -> Option<PathBuf> {
    temp_dir().and_then(|tmp| tmp.parent().map(Path::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandboxes_have_separate_temporary_directories_owned_by_the_command() {
        const CHILD: &str = "THIRDSHIFT_SCRATCH_LIFETIME_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            fs::create_dir(root.path().join("tmp")).unwrap();
            crate::test_support::assert_test_without_warnings(
                "scratch::tests::sandboxes_have_separate_temporary_directories_owned_by_the_command",
                |command| {
                    command
                        .env(CHILD, "1")
                        .env("HOME", root.path().join("home"))
                        .env("TMPDIR", root.path().join("tmp"));
                },
            );
            return;
        }
        let guard = Guard;
        begin(false).unwrap();
        let tmp = temp_dir().unwrap();
        let first = sandbox_temp().unwrap().unwrap();
        let second = sandbox_temp().unwrap().unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(&tmp) && second.starts_with(&tmp));
        fs::write(first.join("private"), "first sandbox").unwrap();
        assert!(!second.join("private").exists());
        drop(guard);
        assert!(!tmp.parent().unwrap().exists());
    }
}
