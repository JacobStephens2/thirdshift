//! Command artifact allocation and the shared legacy/new filename format.
//! A storage owner never changes after allocation, even if its writer fails.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use super::{CommandKind, Work};

pub(super) trait Files {
    type CommandLog: Write + Send;

    /// Exclusively acquire a file, reporting AlreadyExists without changing it.
    fn create_command_log(&mut self, path: &Path) -> io::Result<Self::CommandLog>;
}

const INHERITED_OWNER: &str = "THIRDSHIFT_LOG_OWNER";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Owner {
    stamp: String,
    token: Option<String>,
    command_log: Option<PathBuf>,
}

impl Owner {
    fn identity(&self) -> Identity<'_> {
        Identity {
            stamp: &self.stamp,
            token: self.token.as_deref(),
        }
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.command_log.as_deref()
    }

    pub fn record_path(&self) -> Option<PathBuf> {
        self.command_log
            .as_ref()
            .map(|path| path.with_extension("command.json"))
    }

    pub fn session_path(&self, dir: &Path, subject: &str, kind: &str) -> PathBuf {
        dir.join(format!("{subject}-{}-{kind}.jsonl", self.identity()))
    }

    pub fn models_match(&self, path: &Path) -> bool {
        let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".models.json"))
        else {
            return false;
        };
        let session = path.with_file_name(format!("{stem}.jsonl"));
        decode(&session).is_some_and(|decoded| {
            decoded.session_kind.is_some() && decoded.identity == self.identity()
        })
    }

    /// Only actual child Runs call this. Top-level Commands allocate locally.
    pub fn inherited() -> Result<Self> {
        let value =
            std::env::var(INHERITED_OWNER).context("missing inherited Command log owner")?;
        let owner: Self =
            serde_json::from_str(&value).context("invalid inherited Command log owner")?;
        ensure!(
            parse_stamp(&owner.stamp).is_some(),
            "invalid inherited Command start stamp"
        );
        ensure!(
            owner
                .token
                .as_deref()
                .is_some_and(|token| !token.is_empty()
                    && token.bytes().all(|byte| byte.is_ascii_alphanumeric())),
            "invalid inherited Command owner token"
        );
        if let Some(log) = owner.log_path() {
            let decoded = decode(log).context("invalid inherited Command log path")?;
            ensure!(
                decoded.session_kind.is_none() && decoded.identity == owner.identity(),
                "inherited Command log disagrees with its owner"
            );
        }
        Ok(owner)
    }

    /// Missing ownership explicitly clears stale state on a reused Command.
    pub fn inherit(owner: Option<&Self>, command: &mut std::process::Command) {
        match owner {
            Some(owner) => {
                command.env(
                    INHERITED_OWNER,
                    serde_json::to_string(owner).expect("owner metadata is serializable"),
                );
            }
            None => {
                command.env_remove(INHERITED_OWNER);
            }
        }
    }

    #[cfg(test)]
    pub fn fixture(log: Option<PathBuf>, stamp: String) -> Self {
        let token = log
            .as_deref()
            .and_then(decode)
            .and_then(|decoded| decoded.identity.token)
            .map(str::to_owned);
        Self {
            stamp,
            token,
            command_log: log,
        }
    }
}

pub(super) struct Allocation<W> {
    pub owner: Owner,
    pub writer: W,
}

/// tempfile supplies opaque randomness and collision retries; the adapter
/// supplies exclusive file effects. Only a successful acquisition establishes
/// an owner; a failed candidate must never be exposed through a root record.
pub(super) fn allocate<F: Files>(
    files: &mut F,
    root: &Path,
    work: Work<'_>,
    stamp: &str,
) -> Result<Allocation<F::CommandLog>> {
    let (kind, subject) = match work {
        Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => {
            (CommandKind::Issue, format!("{}-", issue.number))
        }
        Work::PickupRun(listed) => (CommandKind::Pickup, format!("{}-", listed.issue.number)),
        Work::ArchitectRun(_) => (CommandKind::Architect, String::new()),
        Work::SecurityRun(_) => (CommandKind::Secure, String::new()),
    };
    let folder = root.join("commands").join(kind.folder());
    let allocated = tempfile::Builder::new()
        .prefix(&format!("{subject}{stamp}~"))
        .suffix(".log")
        .rand_bytes(16)
        .disable_cleanup(true)
        .make_in(folder, |path| files.create_command_log(path))?;
    let (writer, path) = allocated.into_parts();
    let decoded = decode(&path).context("allocated Command log has an invalid name")?;
    Ok(Allocation {
        owner: Owner {
            stamp: stamp.to_string(),
            token: decoded.identity.token.map(str::to_owned),
            command_log: Some(path.to_path_buf()),
        },
        writer,
    })
}

pub(super) fn create(path: &Path) -> io::Result<File> {
    let create = || {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new().write(true).create_new(true).open(path)
    };
    create().map_err(|error: io::Error| {
        io::Error::new(
            error.kind(),
            format!("could not create {}: {error}", path.display()),
        )
    })
}

pub(super) fn child_record(parent: &Path, dispatch: Option<&str>, issue: u64) -> PathBuf {
    let dispatch = dispatch
        .and_then(|issue| crate::issue::IssueUrl::parse(issue).ok())
        .map_or_else(String::new, |issue| format!("dispatch-{}-", issue.number));
    parent.with_extension(format!("{dispatch}issue-{issue}.child.json"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity<'a> {
    pub stamp: &'a str,
    pub token: Option<&'a str>,
}

impl std::fmt::Display for Identity<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.stamp)?;
        if let Some(token) = self.token {
            write!(formatter, "~{token}")?;
        }
        Ok(())
    }
}

pub(super) struct Decoded<'a> {
    pub subject: &'a str,
    pub identity: Identity<'a>,
    pub session_kind: Option<&'a str>,
}

impl Decoded<'_> {
    pub fn time(&self) -> DateTime<FixedOffset> {
        parse_stamp(self.identity.stamp).expect("decoded stamp is valid")
    }

    pub fn command_log(&self, path: &Path) -> PathBuf {
        let subject = match self.subject {
            "" | "architect" | "secure" => String::new(),
            subject => format!("{subject}-"),
        };
        path.with_file_name(format!("{subject}{}.log", self.identity))
    }
}

fn parse_stamp(stamp: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_str(stamp, "%Y%m%dT%H%M%S%z").ok()
}

/// Read current legacy record suffixes, owned records/logs, and both Session
/// forms. All byte offsets and token syntax live here.
pub(super) fn decode(path: &Path) -> Option<Decoded<'_>> {
    let name = path.file_name()?.to_str()?;
    let (stem, session) = if let Some(stem) = name.strip_suffix(".jsonl") {
        (stem, true)
    } else if let Some(stem) = name.strip_suffix(".log") {
        (stem, false)
    } else {
        (
            name.strip_suffix(".command.json")
                .or_else(|| name.strip_suffix(".child.json"))?
                .split('.')
                .next()?,
            false,
        )
    };
    let (subject, rest) = if stem.get(..20).and_then(parse_stamp).is_some() {
        ("", stem)
    } else {
        stem.split_once('-')?
    };
    let stamp = rest.get(..20)?;
    parse_stamp(stamp)?;
    let tail = rest.get(20..)?;
    let (token, tail) = if let Some(tail) = tail.strip_prefix('~') {
        let end = tail.find('-').unwrap_or(tail.len());
        let token = &tail[..end];
        if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return None;
        }
        (Some(token), &tail[end..])
    } else {
        (None, tail)
    };
    let session_kind = if session {
        Some(tail.strip_prefix('-').filter(|kind| !kind.is_empty())?)
    } else {
        if !tail.is_empty() {
            return None;
        }
        None
    };
    Some(Decoded {
        subject,
        identity: Identity { stamp, token },
        session_kind,
    })
}

/// Preserve current legacy suffix stripping, including non-stamped fixture
/// records, while keeping an owned stem's entire token.
pub(super) fn record_log(path: &Path) -> PathBuf {
    if let Some(decoded) = decode(path) {
        return decoded.command_log(path);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{}.log", name.split('.').next().unwrap_or(&name)))
}

/// Explicit Model evidence can link a transitional tokenless Session to its
/// owned Command. Timestamp equality alone cannot establish that link.
pub(super) fn evidenced_log(session: &Path) -> Option<PathBuf> {
    #[derive(Deserialize)]
    struct Evidence {
        command_log: Option<PathBuf>,
    }
    let bytes = fs::read(session.with_extension("models.json")).ok()?;
    serde_json::from_slice::<Evidence>(&bytes).ok()?.command_log
}

pub(super) fn session_matches(session: &Path, log: &Path) -> bool {
    if let Some(evidence) = evidenced_log(session) {
        return evidence == log;
    }
    match (decode(session), decode(log)) {
        (Some(session), Some(command)) => session.identity == command.identity,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
