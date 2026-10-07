//! A command's record in its repository's logs: its stamp, its Command log
//! and its lines in the Activity log, all under the root of the repository's
//! logs, `<logs.dir>/<owner>/<repo>/`, with its Session logs.
//!
//! A command says how it [`begin`]s, that it is [`configured`], what work it
//! [`started`], what pass was [`skipped`], and how it [`ended`]; this module
//! works out the paths, the order they are written in, and which lines a
//! skipped pass shows. Everything it prints goes through [`print`] and
//! [`eprint`], which keep it in the Command log too.
//!
//! One owned recording implementation holds the complete lifecycle. The
//! process-wide interface serializes its operations, including emitted output.

mod activity;
mod command_log;
mod effects;
mod recording;

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use crate::config::UserConfig;
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};

use activity::Kind;
use effects::OnMachine;
use recording::Record;

/// How a command begins.
pub enum Begin<'a> {
    /// `thirdshift <Issue URL>`, a Run or a Spec run on this issue.
    Run(&'a IssueUrl),
    /// A child Run, a Ticket's Run or a Base fix, with the stamp the command
    /// that started it gave it. It keeps no Command log and writes no
    /// Activity log line: the command that started it does.
    ChildRun(&'a str),
    /// `thirdshift architect`, a pass.
    ArchitectRun,
    /// `thirdshift pickup`, a pass.
    PickupRun,
}

/// The work a command starts.
#[derive(Clone, Copy)]
pub enum Work<'a> {
    /// A Run on its issue.
    Run(&'a IssueUrl),
    /// A Spec run on its Spec.
    SpecRun(&'a IssueUrl),
    /// A Pickup run, on the Ready issue it took.
    PickupRun(&'a IssueUrl),
    /// An Architect run, on its repository.
    ArchitectRun(&'a Repo),
}

/// A pass, a command that may be skipped before it starts work.
#[derive(Clone, Copy)]
pub enum Pass {
    ArchitectRun,
    PickupRun,
}

/// The production command's complete record, shared with reader threads.
static RECORD: LazyLock<Mutex<Record<OnMachine>>> =
    LazyLock::new(|| Mutex::new(Record::new(OnMachine)));

/// Begin the command as `begin` says. A Run or a pass makes its stamp now
/// and prints its first line, dated, as in `starting on <Issue URL>,
/// 2026-10-03 -0400`, then holds its lines for its Command log until it
/// starts work. A pass holds them from the terminal too, until [`configured`]
/// shows them, so a skipped pass may print nothing. A child Run prints no
/// such line, and takes the stamp it was given.
pub fn begin(begin: Begin) {
    record().begin(begin);
}

/// Take `config`'s `logs.dir` as the root of the logs, and, unless its
/// `activity.quiet_skips` is set, show the lines a pass held from the
/// terminal and every line from here on. With it set, a pass's lines stay
/// held until it starts work, ends or fails, so a skipped pass prints
/// nothing.
pub fn configured(config: &UserConfig) {
    record().configured(config);
}

/// Record that the command starts `work`, its sessions on `harness`: keep its
/// Command log, showing any lines held from the terminal, with a line naming
/// the Harness, Model and Effort, then write the Activity log's line that it
/// started, naming that log and those. Only the outermost command's first
/// call does anything: the work a Pickup run or an Architect run started
/// covers the Run it dispatches, and a child Run records nothing, as the
/// command that started it does.
pub fn started(work: Work, harness: &Choice) {
    record().started(work, harness);
}

/// Record that `pass` was skipped on `repo` for `reason`, unless the last
/// Activity log line of its kind there already says so. Nothing is shown:
/// under `activity.quiet_skips`, whatever the pass prints about its skip
/// stays held, and is dropped with the Command log it never kept.
pub fn skipped(pass: Pass, repo: &Repo, reason: impl Display) {
    record().skipped(pass, repo, reason);
}

/// Record how the command ended, `outcome` in short: show any lines held
/// from the terminal, then write the Activity log's end line, if it started
/// work. A skipped pass never calls it, so that under
/// `activity.quiet_skips` its lines stay held.
pub fn ended(outcome: impl Display) {
    record().ended(outcome);
}

/// Print the lines held from the terminal, as before a failure, and print
/// every line from here on.
pub fn show_held() {
    record().show_held();
}

/// The root of `repo`'s logs: `<logs.dir>/<owner>/<repo>/`, named for the
/// GitHub repository, not the checkout, so every checkout of it logs to the
/// same place. Its Command logs, Session logs and Activity log are all under
/// it.
///
/// # Panics
///
/// Before [`configured`], which is a programming error.
pub fn root(repo: &Repo) -> PathBuf {
    record().root(repo)
}

/// The command's start stamp, as in `20261003T120000-0400`, which names its
/// Command log and its Session logs.
pub fn stamp() -> String {
    record().stamp()
}

/// Where the Command log is, if this command created one.
pub fn command_log_path() -> Option<PathBuf> {
    record().command_log_path()
}

/// Print `line` on stderr, and keep it in the Command log.
pub fn eprint(line: &str) {
    record().eprint(line);
}

/// Print `line` on stdout, and keep it in the Command log.
pub fn print(line: &str) {
    record().print(line);
}

/// [`root`], under `logs_dir`.
fn root_under(logs_dir: &Path, repo: &Repo) -> PathBuf {
    logs_dir.join(&repo.owner).join(&repo.name)
}

impl Work<'_> {
    /// The repository whose work it records.
    fn repo(self) -> Repo {
        match self {
            Work::Run(issue) | Work::SpecRun(issue) | Work::PickupRun(issue) => issue.repo(),
            Work::ArchitectRun(repo) => repo.clone(),
        }
    }

    /// Its kind, in the Activity log.
    fn kind(self) -> Kind {
        match self {
            Work::Run(_) => Kind::Run,
            Work::SpecRun(_) => Kind::SpecRun,
            Work::PickupRun(_) => Kind::PickupRun,
            Work::ArchitectRun(_) => Kind::ArchitectRun,
        }
    }

    /// Its issue's number, if it has one.
    fn issue(self) -> Option<u64> {
        match self {
            Work::Run(issue) | Work::SpecRun(issue) | Work::PickupRun(issue) => Some(issue.number),
            Work::ArchitectRun(_) => None,
        }
    }

    /// Where its Command log goes, under its repository's logs at `root`,
    /// for the command started at `stamp`:
    /// `<root>/commands/<folder>/<prefix><stamp>.log`, one folder per
    /// command typed, so the repository, already in the path, is not in the
    /// name. `thirdshift <Issue URL>`, a Run or a Spec run, is in `issue/`,
    /// `thirdshift pickup` in `pickup/`, both named for the issue, and
    /// `thirdshift architect` in `architect/`.
    fn command_log(self, root: &Path, stamp: &str) -> PathBuf {
        let (folder, prefix) = match self {
            Work::Run(issue) | Work::SpecRun(issue) => ("issue", format!("{}-", issue.number)),
            Work::PickupRun(issue) => ("pickup", format!("{}-", issue.number)),
            Work::ArchitectRun(_) => ("architect", String::new()),
        };
        root.join("commands")
            .join(folder)
            .join(format!("{prefix}{stamp}.log"))
    }
}

/// The [`Record`], locked, even if a thread panicked holding it.
fn record() -> MutexGuard<'static, Record<OnMachine>> {
    RECORD.lock().unwrap_or_else(PoisonError::into_inner)
}
