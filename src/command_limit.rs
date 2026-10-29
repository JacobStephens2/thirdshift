//! The Command limit: how many live Commands on one repository on this
//! machine skip a Pass there. Each live Command holds a slot on its
//! repository: an advisory lock on a file under `~/.thirdshift`, held until
//! its process ends, however it ends, so a crash frees it. A Command started
//! on an Issue URL takes a slot without counting. A Pass counts the held
//! slots and takes one under a short lock, so two Passes starting together
//! can't both take the last. The Spec run or Ticket run a Pass dispatches
//! runs inside its process, and so in its slot.
//!
//! A Pass also tries the lock a thirdshift from before the Command limit
//! takes, and keeps it when it is free, so that the two still keep each
//! other out: one held by such a thirdshift counts as a Command running. An Architect run, a
//! Security run or an issue-less Custom process keeps the repository to
//! itself: it is skipped while any other Command runs there, and holds a
//! lock of its own that skips every Pass while it runs.

use std::fmt;
use std::fs::{self, File, TryLockError};
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config;
use crate::issue::Repo;

/// How a Pass shares its repository with the other Commands there.
#[derive(Debug, Clone, Copy)]
pub enum Sharing {
    /// A Pickup run, which runs beside other Commands up to the Command
    /// limit.
    UpToTheLimit,
    /// A Pass that keeps the repository to itself, named as its skip line
    /// names it to another Pass, such as "an Architect run".
    Alone(&'static str),
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
    /// This Pass keeps the repository to itself, and `running` other
    /// Commands are running there.
    NotAlone { running: usize },
    /// A Pass that keeps the repository to itself, named, is running.
    HeldAlone(String),
}

impl fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (repo, limit) = (self.repo.slug(), self.limit);
        // At the default limit every skip says what it said when one Pass at
        // a time held the repository.
        if limit.get() == 1 {
            return write!(f, "another Pass is already running on {repo}");
        }
        match &self.why {
            Why::AtLimit { running } => write!(
                f,
                "at the Command limit on {repo}: {}, pass.command_limit is {limit}",
                commands_running(*running)
            ),
            Why::NotAlone { running } => write!(
                f,
                "{} on {repo}, and this Pass runs only alone there",
                commands_running(*running)
            ),
            Why::HeldAlone(kind) => write!(f, "{kind} is running alone on {repo}"),
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
/// `sharing` says, unless `limit` Commands already run there: `Some` says
/// why it is skipped, and then this call took nothing. Otherwise this
/// process holds the slot until it exits, through whatever it dispatches.
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
    let Some(alone) = slots.try_alone()? else {
        return skipped(Why::HeldAlone(slots.alone_kind()));
    };
    let held = slots.count()?;
    let legacy = try_lock(&legacy_path(repo)?)?;
    let marker = try_lock(&slots.dir.join(LEGACY_HELD))?;
    // A thirdshift from before the Command limit takes no slot, so the lock
    // it takes, held while no Pass from after holds the marker, is one more
    // Command running.
    let running = held.held + usize::from(legacy.is_none() && marker.is_some());
    match sharing {
        Sharing::UpToTheLimit if running >= limit.get() => {
            return skipped(Why::AtLimit { running });
        }
        Sharing::Alone(_) if running >= 1 => return skipped(Why::NotAlone { running }),
        Sharing::UpToTheLimit => drop(alone),
        Sharing::Alone(kind) => {
            let mut alone = alone;
            // What a Pass skipped while this one runs names it as.
            alone
                .set_len(0)
                .and_then(|()| alone.write_all(kind.as_bytes()))
                .with_context(|| format!("can't write {}", slots.dir.join(ALONE).display()))?;
            hold(alone);
        }
    }
    slots.take(held.free)?;
    if let (Some(legacy), Some(marker)) = (legacy, marker) {
        hold(marker);
        hold(legacy);
    }
    Ok(None)
}

/// The lock file a Pass from after the Command limit holds while it holds
/// the lock a thirdshift from before takes, so that the two are told apart.
const LEGACY_HELD: &str = "legacy-held.lock";

/// The lock file that a Pass keeping the repository to itself holds.
const ALONE: &str = "alone.lock";

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
            match try_lock(&path)? {
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
        let slot = try_lock(&path)?
            .with_context(|| format!("{} was taken while counting", path.display()))?;
        hold(slot);
        Ok(())
    }

    fn slot(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{n}.slot"))
    }

    /// The lock that a Pass keeping the repository to itself holds, if no
    /// such Pass holds it now.
    fn try_alone(&self) -> Result<Option<File>> {
        try_lock(&self.dir.join(ALONE))
    }

    /// The name the running Pass that keeps the repository to itself wrote
    /// in its lock, or a general one if it can't be read.
    fn alone_kind(&self) -> String {
        fs::read_to_string(self.dir.join(ALONE))
            .ok()
            .filter(|kind| !kind.is_empty())
            .unwrap_or_else(|| "a Pass".to_string())
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

/// Try, without waiting, to lock `path`: `None` while another process, or
/// another of this process's files, holds it.
fn try_lock(path: &Path) -> Result<Option<File>> {
    let file = open(path)?;
    match file.try_lock() {
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
