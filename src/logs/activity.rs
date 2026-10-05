//! The Activity log: one running record per repository, `activity.log` at
//! the root of its logs, of what the factory did there. A Run, a Spec run,
//! an Architect run or a Pickup run writes a line when it starts work,
//! naming its Command log, and one when it ends, with its outcome. A skipped
//! Architect run or Pickup run writes one only when its reason differs from
//! the last line of its own kind in the file, so a repository that sits idle
//! shows one line, not one per pass. Which work writes its lines is
//! [`super::started`]'s to say.
//!
//! Every line starts with the local date and time, then the kind. Each is
//! appended whole, in one write, so passes on one repository that run at
//! once never interleave within a line. A line that can't be written is one
//! `warning:` line, and changes nothing else.

use std::fmt::{self, Display};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chrono::Local;

use super::command_log;
use crate::harness::Choice;
use crate::progress;

/// The file's name, at the root of a repository's logs.
const FILE: &str = "activity.log";

/// How a line's time is written: local, as in `2026-10-03 10:41:01`.
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// How long a line's time is, as [`TIME_FORMAT`] writes it.
const TIME_FORMAT_LENGTH: usize = "2026-10-03 10:41:01".len();

/// How much of the end of the file is read to find a kind's last line. A
/// pass writes a line or two, so this reaches back over thousands of passes.
const TAIL: u64 = 64 * 1024;

/// What a line is about: the command that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Run,
    SpecRun,
    ArchitectRun,
    PickupRun,
}

impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Kind::Run => "Run",
            Kind::SpecRun => "Spec run",
            Kind::ArchitectRun => "Architect run",
            Kind::PickupRun => "Pickup run",
        })
    }
}

/// The work this command started, once it has: where its end line goes.
struct Started {
    root: PathBuf,
    kind: Kind,
    issue: Option<u64>,
}

/// The work this command started, once [`start`] has recorded it.
static STARTED: OnceLock<Started> = OnceLock::new();

/// Whether the warning that the Activity log can't be written was given.
static WARNED: AtomicBool = AtomicBool::new(false);

/// Record that this command, a `kind` on `issue`, if it has one, starts work
/// on the repository whose logs are at `root`, naming its Command log, if it
/// keeps one, and the Harness, Model and Effort of `harness`, as in
/// `Run #7 started: commands/issue/7-<stamp>.log, on claude · opus · high`.
/// Only once: [`end`] then writes its end line.
pub(super) fn start(root: &Path, kind: Kind, issue: Option<u64>, harness: &Choice) {
    let command_log = match command_log::path() {
        Some(path) => path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string(),
        None => "no Command log".to_string(),
    };
    let started = Started {
        root: root.to_path_buf(),
        kind,
        issue,
    };
    let line = format!("{} started: {command_log}, on {harness}", started.subject());
    if STARTED.set(started).is_ok() {
        write(root, &line, None);
    }
}

/// Record how the work [`start`] recorded ended, `outcome` in short. Nothing
/// if this command started none.
pub(super) fn end(outcome: impl Display) {
    if let Some(started) = STARTED.get() {
        let line = format!("{} ended: {outcome}", started.subject());
        write(&started.root, &line, None);
    }
}

/// Record that this command, a `kind`, was skipped on the repository whose
/// logs are at `root`, for `reason`, unless the last line of its kind there
/// already says so.
pub(super) fn skip(root: &Path, kind: Kind, reason: impl Display) {
    write(root, &format!("{kind} skipped: {reason}"), Some(kind));
}

impl Started {
    /// What its lines start with, after the time: its kind, and its issue.
    fn subject(&self) -> String {
        match self.issue {
            Some(number) => format!("{} #{number}", self.kind),
            None => self.kind.to_string(),
        }
    }
}

/// Append `line`, stamped with the time now, to the Activity log at `root`,
/// creating its folder if missing. With `unless_last_of`, not if it is
/// already the last line of that kind. A failure is the one warning.
fn write(root: &Path, line: &str, unless_last_of: Option<Kind>) {
    let appended = append(&root.join(FILE), line, unless_last_of);
    if let Err(error) = appended
        && !WARNED.swap(true, Ordering::Relaxed)
    {
        // A warning shows even on a pass that holds its lines quiet.
        command_log::show_held();
        progress::step(format_args!(
            "warning: could not keep the Activity log: {error:#}"
        ));
    }
}

/// [`write`], its error returned.
fn append(path: &Path, line: &str, unless_last_of: Option<Kind>) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    if let Some(kind) = unless_last_of {
        let tail = tail(&mut file).with_context(|| format!("could not read {}", path.display()))?;
        if last_of(&tail, kind) == Some(line) {
            return Ok(());
        }
    }
    let stamped = format!("{} {line}\n", Local::now().format(TIME_FORMAT));
    file.write_all(stamped.as_bytes())
        .with_context(|| format!("could not write {}", path.display()))
}

/// The whole lines in the last [`TAIL`] bytes of `file`.
fn tail(file: &mut File) -> std::io::Result<String> {
    let length = file.metadata()?.len();
    let from = length.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(from))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(match (from, text.split_once('\n')) {
        // Read from mid-file: the first line may be cut short.
        (1.., Some((_, rest))) => rest.to_string(),
        _ => text,
    })
}

/// The last line of `kind` in `tail`, without its time.
fn last_of(tail: &str, kind: Kind) -> Option<&str> {
    let kind = kind.to_string();
    tail.lines().rev().find_map(|line| {
        let unstamped = line.get(TIME_FORMAT_LENGTH + 1..)?;
        let rest = unstamped.strip_prefix(&kind)?;
        (rest.starts_with(' ') || rest.starts_with(':')).then_some(unstamped)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kinds_last_line_is_found_past_other_kinds_without_its_time() {
        let tail = "\
2026-10-03 10:00:00 Pickup run skipped: no Ready issue on acme/widgets
2026-10-03 10:00:01 Architect run skipped: a plan is open
2026-10-03 10:01:00 Run #7 started: commands/issue/7-x.log
";
        assert_eq!(
            last_of(tail, Kind::PickupRun),
            Some("Pickup run skipped: no Ready issue on acme/widgets")
        );
        assert_eq!(
            last_of(tail, Kind::ArchitectRun),
            Some("Architect run skipped: a plan is open")
        );
        assert_eq!(
            last_of(tail, Kind::Run),
            Some("Run #7 started: commands/issue/7-x.log")
        );
        assert_eq!(last_of(tail, Kind::SpecRun), None);
    }

    #[test]
    fn a_line_cut_short_or_with_no_time_is_never_a_kinds_last_line() {
        assert_eq!(last_of("Pickup run skipped\n", Kind::PickupRun), None);
        assert_eq!(last_of("", Kind::PickupRun), None);
    }
}
