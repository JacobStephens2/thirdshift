//! The command this process runs, when it is a Run, a Spec run, an Architect
//! run or a Pickup run: the stamp it started at, which names its Command log
//! and every Session log under it, and its Command log, a file of everything
//! it printed, stderr and stdout in the order printed. The terminal shows
//! everything as it would without one.
//!
//! The process the user or cron started keeps the Command log. A child Run
//! keeps none, but takes the stamp of the command that started it: its lines
//! reach that command's stderr through the relay, and from there its Command
//! log. A command's lines are held from its start until the file's name is
//! known, then written first, so a command that ends before then, as a
//! skipped Pickup run does, never creates the file. A Command log that can't
//! be written is one `warning:` line, and changes nothing else.
//!
//! An Architect run or a Pickup run, which may yet be skipped, holds its
//! lines from the terminal too, until it knows it will print them: a
//! skipped pass with `activity.quiet_skips` set prints nothing at all.

use std::fmt::Display;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

use anyhow::{Context, Result};
use chrono::Local;

use crate::issue::{IssueUrl, Repo};
use crate::progress;

/// How a stamp is written: local time with its UTC offset, as in
/// `20261003T120000-0400`.
const STAMP_FORMAT: &str = "%Y%m%dT%H%M%S%z";

/// The command's start stamp, made once.
static STARTED: OnceLock<String> = OnceLock::new();

/// The Command log, if this command keeps one.
static LOG: Mutex<Log> = Mutex::new(Log::NotKept);

/// Where a Command log stands.
enum Log {
    /// The command keeps none: a child Run, or a command such as `setup`.
    NotKept,
    /// Its lines so far, held until its name is known, each with the
    /// stream it is for, and whether they have been printed there yet.
    Held {
        lines: Vec<(Stream, String)>,
        shown: bool,
    },
    /// Being written to `file`, at `path`.
    Kept { path: PathBuf, file: File },
    /// It could not be written, and the warning has been given. `path` is
    /// the file, if it was created.
    Failed { path: Option<PathBuf> },
}

/// Begin a command that keeps a Command log: make its stamp now, hold its
/// lines from here on, and print its first progress line, `<starting>, `
/// followed by the date and UTC offset, as in `Pickup run starting,
/// 2026-10-03 -0400`.
pub fn begin(starting: impl Display) {
    begin_held(starting, true);
}

/// Begin a pass, an Architect run or a Pickup run, as [`begin`] does, but
/// hold its lines from the terminal too, until [`show_held`] or [`keep`]
/// prints them: one that is skipped may print nothing.
pub fn begin_pass(starting: impl Display) {
    begin_held(starting, false);
}

/// [`begin`], its lines already `shown` on the terminal or held from it.
fn begin_held(starting: impl Display, shown: bool) {
    let now = Local::now();
    let _ = STARTED.set(now.format(STAMP_FORMAT).to_string());
    *log() = Log::Held {
        lines: Vec::new(),
        shown,
    };
    progress::step(format_args!("{starting}, {}", now.format("%Y-%m-%d %z")));
}

/// Print the lines a pass [`begin_pass`] began has held from the terminal,
/// and print its lines as they come from here on.
pub fn show_held() {
    if let Log::Held { lines, shown } = &mut *log()
        && !*shown
    {
        for (stream, line) in lines.iter() {
            stream.print(line);
        }
        *shown = true;
    }
}

/// Begin a child Run, which keeps no Command log, with the `stamp` the
/// command that started it gave it, if it gave one.
pub fn begin_child(stamp: Option<String>) {
    if let Some(stamp) = stamp {
        let _ = STARTED.set(stamp);
    }
}

/// The command's start stamp, as in `20261003T120000-0400`: made by
/// [`begin`], given by [`begin_child`], or else made now.
pub fn stamp() -> &'static str {
    STARTED.get_or_init(|| Local::now().format(STAMP_FORMAT).to_string())
}

/// The root of `repo`'s logs under `logs_dir`, the User config's
/// `logs.dir`: `<logs_dir>/<owner>/<repo>/`, named for the GitHub repository,
/// not the checkout, so every checkout of it logs to the same place. Its
/// Command logs, Session logs and Activity log are all under it.
pub fn root(logs_dir: &Path, repo: &Repo) -> PathBuf {
    logs_dir.join(&repo.owner).join(&repo.name)
}

/// Where the Command log of `thirdshift <Issue URL>` on `issue` goes, under
/// `logs_dir`.
pub fn of_run(logs_dir: &Path, issue: &IssueUrl) -> PathBuf {
    of(
        logs_dir,
        &issue.repo(),
        "issue",
        &format!("{}-", issue.number),
    )
}

/// Where the Command log of a Pickup run that took `issue` goes, under
/// `logs_dir`.
pub fn of_pickup_run(logs_dir: &Path, issue: &IssueUrl) -> PathBuf {
    of(
        logs_dir,
        &issue.repo(),
        "pickup",
        &format!("{}-", issue.number),
    )
}

/// Where the Command log of an Architect run on `repo` goes, under
/// `logs_dir`.
pub fn of_architect_run(logs_dir: &Path, repo: &Repo) -> PathBuf {
    of(logs_dir, repo, "architect", "")
}

/// `<root>/commands/<folder>/<prefix><stamp>.log`, under `repo`'s [`root`]:
/// one folder per command typed, so the repository, already in the path,
/// is not in the name.
fn of(logs_dir: &Path, repo: &Repo, folder: &str, prefix: &str) -> PathBuf {
    root(logs_dir, repo)
        .join("commands")
        .join(folder)
        .join(format!("{prefix}{}.log", stamp()))
}

/// Create the Command log at `path`, its folder too if missing, with the
/// lines held so far, and say where it is. Only a command [`begin`] began,
/// and only once. One that can't be created is a warning, and the command
/// carries on without it.
pub fn keep(path: PathBuf) {
    show_held();
    let created = {
        let mut log = log();
        let Log::Held { lines, .. } = &*log else {
            return;
        };
        let created = create(&path, lines.iter().map(|(_, line)| line));
        *log = match &created {
            Ok(file) => Log::Kept {
                path: path.clone(),
                file: file.try_clone().expect("a created file can be cloned"),
            },
            // A file created but not written is still where to look.
            Err(_) => Log::Failed {
                path: path.exists().then(|| path.clone()),
            },
        };
        created
    };
    match created {
        Ok(_) => progress::step(format_args!("logging this command to {}", path.display())),
        Err(error) => warn(&error),
    }
}

/// The file at `path`, created with its folder, and the `held` lines written.
fn create<'a>(path: &Path, held: impl Iterator<Item = &'a String>) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut file =
        File::create(path).with_context(|| format!("could not create {}", path.display()))?;
    for line in held {
        writeln!(file, "{line}").with_context(|| format!("could not write {}", path.display()))?;
    }
    Ok(file)
}

/// Where the Command log is, if this command created one.
pub fn path() -> Option<PathBuf> {
    match &*log() {
        Log::Kept { path, .. } => Some(path.clone()),
        Log::Failed { path } => path.clone(),
        Log::NotKept | Log::Held { .. } => None,
    }
}

/// Print `line` on stderr, and keep it in the Command log.
pub fn eprint(line: &str) {
    print_to(Stream::Stderr, line);
}

/// Print `line` on stdout, and keep it in the Command log.
pub fn print(line: &str) {
    print_to(Stream::Stdout, line);
}

/// Where on the terminal a line goes.
#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    /// Print `line` here. Ignored if it fails, as it does once the terminal
    /// has closed: the Run still has to clean up and send its Run
    /// notification.
    fn print(self, line: &str) {
        fn to(mut terminal: impl Write, line: &str) {
            let _ = writeln!(terminal, "{line}");
            let _ = terminal.flush();
        }
        match self {
            Stream::Stdout => to(std::io::stdout(), line),
            Stream::Stderr => to(std::io::stderr(), line),
        }
    }
}

/// Print `line` to `stream`, unless it is held from the terminal, and keep it
/// in the Command log, both under the one lock, so the file has the lines in
/// the order they were printed.
fn print_to(stream: Stream, line: &str) {
    let failed = {
        let mut log = log();
        if !matches!(&*log, Log::Held { shown: false, .. }) {
            stream.print(line);
        }
        match &mut *log {
            Log::NotKept | Log::Failed { .. } => None,
            Log::Held { lines, .. } => {
                lines.push((stream, line.to_string()));
                None
            }
            Log::Kept { path, file } => match writeln!(file, "{line}") {
                Ok(()) => None,
                Err(error) => {
                    let error = anyhow::Error::new(error)
                        .context(format!("could not write {}", path.display()));
                    *log = Log::Failed {
                        path: Some(path.clone()),
                    };
                    Some(error)
                }
            },
        }
    };
    if let Some(error) = failed {
        warn(&error);
    }
}

/// The one warning that the Command log can't be written, for `error`.
fn warn(error: &anyhow::Error) {
    progress::step(format_args!(
        "warning: could not keep the Command log: {error:#}"
    ));
}

/// The Command log, locked, even if a thread panicked holding it.
fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(PoisonError::into_inner)
}
