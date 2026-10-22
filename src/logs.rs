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
mod command_record;
pub(crate) mod convert;
mod effects;
pub mod export;
pub(crate) mod owner;
mod process_identity;
mod reading;
pub(crate) use reading::catalog;
mod recording;
pub(crate) mod session;
pub(crate) mod spans;
pub mod status;
pub mod stop;
pub(crate) mod trace;

use std::fmt::{self, Display};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use crate::config::UserConfig;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::session::Logged;

use activity::Kind;
pub use command_record::StageName as Stage;
pub(crate) use command_record::{TicketRecord, TicketState};
use effects::OnMachine;
use recording::Record;

/// The command typed, which owns its Session logs and all child Runs' logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Issue,
    Architect,
    Pickup,
    Secure,
}

impl CommandKind {
    pub fn folder(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Architect => "architect",
            Self::Pickup => "pickup",
            Self::Secure => "secure",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "issue" => Some(Self::Issue),
            "architect" => Some(Self::Architect),
            "pickup" => Some(Self::Pickup),
            "secure" => Some(Self::Secure),
            _ => None,
        }
    }
}

/// How a command begins.
pub enum Begin<'a> {
    /// `thirdshift <Issue URL>`, a Run or a Spec run on this issue.
    Run(&'a IssueUrl),
    /// A child Run, a Ticket's Run or a Base fix, with the stamp the command
    /// that started it gave it. It keeps no Command log and writes no
    /// Activity log line: the command that started it does.
    ChildRun(&'a str, CommandKind),
    /// `thirdshift architect`, a pass.
    ArchitectRun,
    /// `thirdshift pickup`, a pass.
    PickupRun,
    SecurityRun,
}

/// The work a command starts.
#[derive(Clone, Copy)]
pub enum Work<'a> {
    /// A Run on its issue.
    Run(&'a IssueUrl),
    /// A Base fix, a child Run into the Run's Base branch.
    BaseFix(&'a IssueUrl),
    /// A Spec run on its Spec.
    SpecRun(&'a IssueUrl),
    /// A Pickup run, on the Ready issue it took, as the listing named it.
    PickupRun(&'a ListedIssue),
    /// An Architect run, on its repository.
    ArchitectRun(&'a Repo),
    /// A Security run, on its repository.
    SecurityRun(&'a Repo),
}

/// A Pass, a command that may be skipped before it starts work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Architect,
    Pickup,
    Security,
}

impl Display for Pass {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Pass::Architect => "Architect run",
            Pass::Pickup => "Pickup run",
            Pass::Security => "Security run",
        })
    }
}

/// The production command's complete record, shared with reader threads.
static RECORD: LazyLock<Mutex<Record<OnMachine>>> =
    LazyLock::new(|| Mutex::new(Record::new(OnMachine)));

const PARENT_RECORD: &str = "THIRDSHIFT_PARENT_RECORD";
const PARENT_DISPATCH: &str = "THIRDSHIFT_PARENT_DISPATCH";

/// Begin the command as `begin` says. A Run or a pass makes its stamp now
/// and prints its first line, dated, as in `starting on <Issue URL>,
/// 2026-10-03 -0400`, then holds its lines for its Command log until it
/// starts work. A pass holds them from the terminal too, until [`configured`]
/// shows them, so a skipped pass may print nothing. A child Run prints no
/// such line, and takes the stamp it was given.
pub fn begin(begin: Begin) {
    let mut record = record();
    if matches!(begin, Begin::ChildRun(..)) {
        record.inherit_owner(owner::Owner::inherited());
        record.child_of(std::env::var_os(PARENT_RECORD).map(PathBuf::from));
        record.child_dispatch(std::env::var(PARENT_DISPATCH).ok());
    }
    record.begin(begin);
}

/// Give a child Run its exact parent, independently of second-resolution stamps.
pub fn inherit_record(command: &mut std::process::Command) {
    if let Some(path) = record().record_path() {
        command.env(PARENT_RECORD, path);
    } else {
        command.env_remove(PARENT_RECORD);
    }
    if let Some(issue) = record().dispatch_issue() {
        command.env(PARENT_DISPATCH, issue);
    } else {
        command.env_remove(PARENT_DISPATCH);
    }
}

/// Replace the Spec run's Ticket accounting without touching child-owned files.
pub(crate) fn tickets(tickets: Vec<TicketRecord>) {
    record().tickets(tickets);
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
/// call keeps the logs. A Pass also records the Run or Spec run it dispatches
/// beneath itself; each child process keeps its own record linked to its
/// immediate parent, with no Command log of its own.
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

/// Add the Run's title once its work has started.
pub fn issue_title(title: Option<String>) {
    record().issue_title(title);
}

/// Begin a stage, retaining its start time until the next transition.
pub fn stage(stage: Stage) {
    record().stage(stage, None);
}

/// Keep the current Session log, including a Resume's own log, as
/// `session` enters its stage.
pub fn session(session: Logged, path: &Path) {
    record().stage(session.stage(), Some(path));
}

/// The selected Issue branch, retained unless completion says it was removed.
pub fn issue_branch(branch: Option<&str>) {
    record().issue_branch(branch);
}

/// Complete the durable facts from the same account used by other renderers.
pub fn finished(account: &crate::run_ending::Account<'_>) {
    record().finished(account);
}

/// The Stop given to this Command, if interruption is being handled.
pub fn stop_requested() -> Option<stop::Request> {
    if !crate::interrupt::requested() {
        return None;
    }
    record().stop_request()
}

/// Finish a Pass's dispatched Run without ending its owning Command.
pub fn dispatch_ended(ended: &crate::run::Ended) {
    record().dispatch_ended(&crate::run_ending::Account::of_run(ended));
}

/// Keep a Pass's dispatch before the Run's fallible launch checks.
pub fn dispatching(work: Work, harness: &Choice) {
    record().dispatching(work, harness);
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

/// The command whose folder holds this process's Session logs.
pub fn command_kind() -> CommandKind {
    record().command_kind()
}

/// Where this command and all its child Runs keep Session logs on `repo`.
pub fn session_dir(repo: &Repo) -> PathBuf {
    let record = record();
    record
        .root(repo)
        .join("commands")
        .join(record.command_kind().folder())
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

pub(crate) fn captured_owner() -> Result<owner::Owner, String> {
    record().captured_owner()
}

pub(crate) fn trace_context() -> Option<trace::Context> {
    record().trace_context()
}

pub(crate) fn otlp_endpoint() -> Option<String> {
    record().otlp_endpoint()
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

impl<'a> Work<'a> {
    fn initial_stage(self) -> Option<Stage> {
        match self {
            Work::Run(_) | Work::BaseFix(_) => Some(Stage::Implement),
            Work::SpecRun(_) => Some(Stage::Tickets),
            Work::ArchitectRun(_) => Some(Stage::ArchitectureReview),
            Work::PickupRun(_) | Work::SecurityRun(_) => None,
        }
    }

    /// The repository whose work it records.
    fn repo(self) -> Repo {
        match self {
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => issue.repo(),
            Work::PickupRun(listed) => listed.issue.repo(),
            Work::ArchitectRun(repo) | Work::SecurityRun(repo) => repo.clone(),
        }
    }

    /// Its kind, in the Activity log.
    fn kind(self) -> Kind {
        match self {
            Work::Run(_) | Work::BaseFix(_) => Kind::Run,
            Work::SpecRun(_) => Kind::SpecRun,
            Work::PickupRun(_) => Kind::Pass(Pass::Pickup),
            Work::ArchitectRun(_) => Kind::Pass(Pass::Architect),
            Work::SecurityRun(_) => Kind::Pass(Pass::Security),
        }
    }

    /// Its issue, if it has one.
    pub(crate) fn issue_url(self) -> Option<&'a IssueUrl> {
        match self {
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => Some(issue),
            Work::PickupRun(listed) => Some(&listed.issue),
            Work::ArchitectRun(_) | Work::SecurityRun(_) => None,
        }
    }

    /// Its issue's number, if it has one.
    fn issue(self) -> Option<u64> {
        self.issue_url().map(|issue| issue.number)
    }
}

/// The [`Record`], locked, even if a thread panicked holding it.
fn record() -> MutexGuard<'static, Record<OnMachine>> {
    RECORD.lock().unwrap_or_else(PoisonError::into_inner)
}
