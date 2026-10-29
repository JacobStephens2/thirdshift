//! The Command limit: how many live Commands on one repository on this
//! machine skip a Pass there. Each live Command holds a slot on its
//! repository: an advisory lock on a file under `~/.thirdshift`, held until
//! its process ends, however it ends, so a crash frees it. A Command started
//! on an Issue URL takes a slot without counting. A Pass counts the held
//! slots and takes one under a short lock, so two Passes starting together
//! can't both take the last. The Spec run or Ticket run a Pass dispatches
//! runs inside its process, and so in its slot.
//!
//! Passes of different kinds share the repository up to the limit, but an
//! Architect run and a Security run each hold a lock of their kind too, and
//! are skipped while another of their kind holds it: never two of a kind on
//! one repository at once. A Pickup run or an issue-less Custom process
//! takes no lock of a kind.
//!
//! A Pass also tries a shared lock on the file a thirdshift from before the
//! Command limit locks exclusively. Every overlapping newer Pass keeps its
//! shared lock until it ends, so an older Pass cannot start until they all
//! end. One held by an older thirdshift counts as a Command running and
//! excludes every newer Pass, even above a Command limit of one.

use std::fmt;
use std::fs::{self, File, TryLockError};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config;
use crate::issue::Repo;

/// How a Pass shares its repository with the other Commands there.
#[derive(Debug, Clone, Copy)]
pub enum Sharing {
    /// A Pickup run, or a Custom process started with no issue, which runs
    /// beside other Commands up to the Command limit.
    UpToTheLimit,
    /// A Pass that runs beside other Commands up to the Command limit, but
    /// never beside another of its kind.
    OneOfAKind(PassKind),
}

/// A kind of Pass of which only one runs on a repository at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Architect,
    Security,
}

impl PassKind {
    /// What it is named as in a skip's reason, such as "an Architect run".
    fn name(self) -> &'static str {
        match self {
            PassKind::Architect => "an Architect run",
            PassKind::Security => "a Security run",
        }
    }

    /// The slug its lock file and its review worktree are named with.
    pub fn slug(self) -> &'static str {
        match self {
            PassKind::Architect => "architect",
            PassKind::Security => "security",
        }
    }
}

/// Why a Pass is skipped at its start, on `repo`, at the Command `limit`.
/// Its `Display` is the reason, as the skipped run gives it.
#[derive(Debug)]
pub struct AlreadyRunning {
    pub repo: Repo,
    limit: NonZeroUsize,
    why: Why,
}

#[derive(Debug)]
enum Why {
    /// `running` Commands are running, at or over the limit.
    AtLimit { running: usize },
    /// Another Pass of this one's kind is running.
    SameKind(PassKind),
    /// A thirdshift from before the Command limit is running an exclusive
    /// Pass, even though the Command limit would admit another.
    Older,
}

impl fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (repo, limit) = (self.repo.slug(), self.limit);
        match self.why {
            Why::AtLimit { running } => write!(
                f,
                "at the Command limit on {repo}: {}, pass.command_limit is {limit}",
                commands_running(running)
            ),
            Why::SameKind(kind) => write!(f, "{} is already running on {repo}", kind.name()),
            Why::Older => write!(f, "an older thirdshift's Pass is running on {repo}"),
        }
    }
}

/// "1 Command is running", or "`n` Commands are running".
fn commands_running(n: usize) -> String {
    match n {
        1 => "1 Command is running".to_string(),
        n => format!("{n} Commands are running"),
    }
}

/// Take a slot on `repo` for this process, a Command started with an issue,
/// without counting the slots held: such a Command is never refused. It
/// holds the slot until it exits. One it can't take is a warning: the
/// Command runs all the same, only Passes don't count it.
pub fn hold_slot(repo: &Repo) {
    let taken = (|| {
        let slots = Slots::of(repo)?;
        let _counting = slots.counting()?;
        let free = slots.count()?.free;
        slots.take(free)
    })();
    if let Err(error) = taken {
        crate::progress::warn(
            &error,
            format_args!("this Command does not count toward the Command limit"),
        );
    }
}

/// Take a slot on `repo` for this process, a Pass that shares it as
/// `sharing` says, unless `limit` Commands already run there, or another of
/// its kind: `Some` says why it is skipped, and then this call took
/// nothing. Otherwise this process holds the slot, and the lock of its kind
/// if it has one, until it exits, through whatever it dispatches.
pub fn hold_pass(
    repo: &Repo,
    limit: NonZeroUsize,
    sharing: Sharing,
) -> Result<Option<AlreadyRunning>> {
    let slots = Slots::of(repo)?;
    let _counting = slots.counting()?;
    let skipped = |why| {
        Ok(Some(AlreadyRunning {
            repo: repo.clone(),
            limit,
            why,
        }))
    };
    let kind = match sharing {
        Sharing::UpToTheLimit => None,
        Sharing::OneOfAKind(kind) => match slots.try_kind(kind)? {
            Some(lock) => Some((kind, lock)),
            None => return skipped(Why::SameKind(kind)),
        },
    };
    let held = slots.count()?;
    let legacy = try_lock(&legacy_path(repo)?, File::try_lock_shared)?;
    // A thirdshift from before the Command limit takes no slot, so the lock
    // it holds exclusively is one more Command running. Newer Passes share
    // that lock, so each keeps an older Pass out until its own process ends.
    let running = held.held + usize::from(legacy.is_none());
    if running >= limit.get() {
        return skipped(Why::AtLimit { running });
    }
    // An older Pass expects exclusive access. Starting beside it would also
    // leave this Pass unable to keep another older Pass out after it ends.
    let Some(legacy) = legacy else {
        return skipped(Why::Older);
    };
    slots.take(held.free)?;
    if let Some((_, lock)) = kind {
        hold(lock);
    }
    hold(legacy);
    Ok(None)
}

/// The slots of one repository: a directory under `~/.thirdshift` named for
/// it, as GitHub compares names, whatever their case. Each slot is a file
/// `<n>.slot` there, held while locked, and meaning nothing unless locked;
/// none is ever deleted, so there are only ever as many as the most Commands
/// that ran at once.
struct Slots {
    dir: PathBuf,
}

/// What [`Slots::count`] found: how many slots are held, and the lowest
/// numbered free one, which may be one past the last file.
struct Count {
    held: usize,
    free: usize,
}

impl Slots {
    fn of(repo: &Repo) -> Result<Self> {
        let dir = config::home()?
            .join(".thirdshift/command-slots")
            .join(repo.owner.to_ascii_lowercase())
            .join(repo.name.to_ascii_lowercase());
        fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
        Ok(Slots { dir })
    }

    /// The short lock under which slots are counted and taken, held until
    /// the file returned is closed. It waits for any other holder, which
    /// holds it only for as long as counting takes.
    fn counting(&self) -> Result<File> {
        let path = self.dir.join("count.lock");
        let file = open(&path)?;
        file.lock()
            .with_context(|| format!("can't lock {}", path.display()))?;
        Ok(file)
    }

    /// Count the held slots, trying each slot file there is: one this
    /// process can lock is free, and is unlocked again as its file closes.
    fn count(&self) -> Result<Count> {
        let mut count = Count { held: 0, free: 0 };
        let mut free = None;
        for n in 0.. {
            let path = self.slot(n);
            if !path.exists() {
                count.free = free.unwrap_or(n);
                return Ok(count);
            }
            match try_lock(&path, File::try_lock)? {
                Some(_) => free = free.or(Some(n)),
                None => count.held += 1,
            }
        }
        unreachable!("the slots ran out")
    }

    /// Lock slot `n`, found free while counting, and hold it until this
    /// process exits.
    fn take(&self, n: usize) -> Result<()> {
        let path = self.slot(n);
        let slot = try_lock(&path, File::try_lock)?
            .with_context(|| format!("{} was taken while counting", path.display()))?;
        hold(slot);
        Ok(())
    }

    fn slot(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{n}.slot"))
    }

    /// The lock a Pass of `kind` holds while it runs, if no such Pass
    /// holds it now.
    fn try_kind(&self, kind: PassKind) -> Result<Option<File>> {
        try_lock(
            &self.dir.join(format!("{}.lock", kind.slug())),
            File::try_lock,
        )
    }
}

/// The lock a thirdshift from before the Command limit took for its one
/// Pass on `repo`. Its directory keeps the name it had when only an
/// Architect run took the lock, so that a thirdshift from before the Pickup
/// run still sees a Pass running too.
fn legacy_path(repo: &Repo) -> Result<PathBuf> {
    let dir = config::home()?
        .join(".thirdshift/architect-locks")
        .join(repo.owner.to_ascii_lowercase());
    fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
    Ok(dir.join(format!("{}.lock", repo.name.to_ascii_lowercase())))
}

/// `path`, created if missing, opened to lock and to write, never truncated
/// on opening, so a holder's contents stay readable.
fn open(path: &Path) -> Result<File> {
    File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("can't open {}", path.display()))
}

/// Try, without waiting, to lock `path` using the shared or exclusive
/// `locking` operation: `None` while an incompatible lock is held.
fn try_lock(path: &Path, locking: fn(&File) -> Result<(), TryLockError>) -> Result<Option<File>> {
    let file = open(path)?;
    match locking(&file) {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("can't lock {}", path.display()))
        }
    }
}

/// Keep `lock` held for as long as this process lives: it is never closed,
/// and the operating system releases it however the process ends.
fn hold(lock: File) {
    std::mem::forget(lock);
}
