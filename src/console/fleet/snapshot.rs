//! Where a watching Console keeps what it last saw of a Fleet machine: one
//! file per machine and slot, either one person's reads of it or what the
//! machine reported itself under a `report` token. Fleet metrics and Fleet
//! Plan usage each keep their own directory of these files.
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// How a Fleet view came by a machine's answer, or why it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Status {
    Ok,
    /// Unreachable now, or no longer in the Fleet: its saved snapshot shows.
    Cached,
    /// Not read from here: the summary it last pushed shows, read-only.
    Reported,
    Unavailable,
    Refused,
    Incompatible,
    LinkOnly,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ok => "ok",
            Self::Cached => "cached",
            Self::Reported => "reported",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
            Self::Incompatible => "incompatible",
            Self::LinkOnly => "link-only",
        })
    }
}

/// Which saved snapshot of a machine: one person's reads, or what the
/// machine reported itself. A peer answers each person by its own sign-in
/// list, so a read stands in only for the person whose read it keeps;
/// missing for this machine's owner. A reported snapshot stands in for
/// everyone who can read this Console.
#[derive(Clone, Copy)]
pub(crate) enum Slot<'a> {
    Read(Option<&'a str>),
    Reported,
}

/// What a saved snapshot says about whose it is.
pub(crate) trait Held {
    fn machine(&self) -> &str;
    fn person(&self) -> Option<&str>;
    /// Whether the machine pushed it itself.
    fn pushed(&self) -> bool;
}

impl<T: Held> Held for &T {
    fn machine(&self) -> &str {
        (**self).machine()
    }

    fn person(&self) -> Option<&str> {
        (**self).person()
    }

    fn pushed(&self) -> bool {
        (**self).pushed()
    }
}

impl<'a> Slot<'a> {
    /// Whose reads the slot keeps; a reported slot keeps no person's.
    pub(crate) fn person(self) -> Option<&'a str> {
        match self {
            Self::Read(person) => person,
            Self::Reported => None,
        }
    }

    pub(crate) fn reported(self) -> bool {
        matches!(self, Self::Reported)
    }

    /// Whether `saved` is `name`'s snapshot in this slot, and not a file
    /// that merely sits at its path.
    pub(crate) fn holds(self, saved: &impl Held, name: &str) -> bool {
        saved.machine() == name
            && saved.pushed() == self.reported()
            && saved.person() == self.person()
    }

    /// A name's saved snapshot file in this slot, under `cache` in the
    /// logs. Names are printable but may hold any character, so the file is
    /// named by a digest; the file names the machine, and the person or that
    /// the machine reported it.
    pub(crate) fn path(self, logs_dir: &Path, cache: &str, name: &str) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(name.as_bytes());
        match self {
            Self::Read(None) => {}
            Self::Read(Some(person)) => {
                digest.update([0]);
                digest.update(person.as_bytes());
            }
            // No printable name or person holds a 1 byte.
            Self::Reported => digest.update([1]),
        }
        let digest = digest.finalize();
        let hex: String = digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        logs_dir.join(cache).join(format!("{hex}.json"))
    }
}
