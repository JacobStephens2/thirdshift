//! A command's record in its repository's logs: its stamp, its Command log
//! and its lines in the Activity log, all under the root of the repository's
//! logs, `<logs.dir>/<owner>/<repo>/`, with its Session traces.
//!
//! A command says how it [`begin`]s, that it is [`configured`], what work it
//! [`started`], what pass was [`skipped`], and how it [`ended`]; this module
//! works out the paths, the order they are written in, and which lines a
//! skipped pass shows. Everything it prints goes through [`print`] and
//! [`eprint`], which keep it in the Command log too.
//!
//! One owned recording implementation holds the complete lifecycle. The
//! process-wide interface serializes its operations, including emitted output.

pub(crate) mod accounts;
mod actor;
pub(crate) use actor::Actor;
mod activity;
mod command_record;
pub mod dismiss;
mod effects;
mod live;
pub(crate) use live::Command as LiveCommand;
pub mod export;
pub(crate) mod github;
pub mod metrics;
pub(crate) mod owner;
pub(crate) mod pass_starts;
mod process_identity;
mod reading;
pub(crate) use reading::Framing;
pub(crate) use reading::catalog;
pub(crate) use reading::run_for_pull_request;
mod recording;
mod run_metrics;
pub(crate) mod session;
pub(crate) mod session_signals;
pub(crate) mod spans;
pub(crate) mod start_notice;
pub mod status;
pub mod stop;
pub(crate) mod trace;
mod usual_range;

use std::fmt::{self, Display};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, Once, PoisonError};

use crate::config::UserConfig;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::session::Logged;

use activity::Kind;
pub use command_record::StageName as Stage;
pub(crate) use command_record::{
    FailureCategory, Issue, TicketRecord, TicketState, experiment_rechecked,
};
use effects::OnMachine;
use recording::Record;

/// Live Commands of this OS account, across repositories, excluding this process.
pub(crate) fn live_commands() -> anyhow::Result<Vec<LiveCommand>> {
    // An issue-targeted Custom process takes its slot before recording begins.
    let configured = record().logs_directory();
    let root = match configured {
        Some(root) => root,
        None => UserConfig::load()?.logs_dir,
    };
    live::read(&root)
}

/// Find only a retained record owned by the process this Console just spawned.
pub(crate) fn command_for_process(
    config: &UserConfig,
    repo: &Repo,
    process_id: u32,
    started_after: chrono::DateTime<chrono::Local>,
) -> anyhow::Result<Option<String>> {
    Ok(
        reading::repository_records(&root_under(&config.logs_dir, repo))?
            .into_values()
            .find(|tree| {
                tree.command.process_id == process_id
                    && tree.command.parent_record.is_none()
                    && chrono::DateTime::parse_from_rfc3339(&tree.command.started_at)
                        .is_ok_and(|start| start >= started_after)
            })
            .map(|tree| tree.command.name),
    )
}

/// The Unix actor recorded for a CLI action, shared by Stop and Dismissal.
fn unix_user(action: &str) -> anyhow::Result<String> {
    use anyhow::Context;

    let failure = format!("could not identify the Unix user asking for {action}");
    let user = std::process::Command::new("id")
        .arg("-un")
        .output()
        .with_context(|| failure.clone())?;
    if !user.status.success() {
        anyhow::bail!(failure);
    }
    Ok(String::from_utf8(user.stdout)?.trim().to_owned())
}

/// Human-facing names for stable stored kinds and dispatch stages (ADR 0022).
pub(crate) fn display_name(stored: &str) -> &str {
    match stored {
        "Run" => "Ticket run",
        "dispatched Run" => "dispatched Ticket run",
        other => other,
    }
}

/// The command typed, which owns its Session traces and all child Runs' logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Issue,
    Architect,
    Pickup,
    Secure,
    Scoring,
    Triage,
    Experiment,
    Process,
    Review,
    Fix,
}

impl CommandKind {
    pub fn folder(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Architect => "architect",
            Self::Pickup => "pickup",
            Self::Secure => "secure",
            Self::Scoring => "scoring",
            Self::Triage => "triage",
            Self::Experiment => "experiment",
            Self::Process => "process",
            Self::Review => "review",
            Self::Fix => "fix",
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        match name {
            "issue" => Some(Self::Issue),
            "architect" => Some(Self::Architect),
            "pickup" => Some(Self::Pickup),
            "secure" => Some(Self::Secure),
            "scoring" => Some(Self::Scoring),
            "triage" => Some(Self::Triage),
            "experiment" => Some(Self::Experiment),
            "process" => Some(Self::Process),
            "review" => Some(Self::Review),
            "fix" => Some(Self::Fix),
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
    ScoringRun,
    ReviewRun {
        pass: bool,
    },
    FixRun {
        pass: bool,
    },
    TriageRun {
        pass: bool,
    },
    Experiment,
    /// `thirdshift process <name> [issue <issue>]`, a Custom process.
    /// `pass` holds the first lines, as the other Passes do, when it was
    /// started with no issue.
    Process {
        name: &'a str,
        pass: bool,
    },
}

/// The work a command starts.
#[derive(Clone, Copy)]
pub enum Work<'a> {
    /// A Command still in preflight, retained only because it hit a Quota wait.
    Starting {
        repo: &'a Repo,
        command: CommandKind,
        issue: Option<&'a IssueUrl>,
    },
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
    ScoringRun(&'a Repo),
    ReviewRun(&'a Repo),
    FixRun(&'a Repo),
    /// Triage on a Report, or on the retained explicit or Pass-selected issue.
    TriageRun {
        repo: &'a Repo,
        issue: Option<&'a IssueUrl>,
    },
    Experiment(&'a Repo),
    /// A Custom process, named by its file.
    Process {
        repo: &'a Repo,
        name: &'a str,
    },
}

/// A Pass, a command that may be skipped before it starts work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pass {
    Architect,
    Pickup,
    Security,
    Review,
    Fix,
    Triage,
}

impl Pass {
    const ALL: [Self; 6] = [
        Self::Architect,
        Self::Pickup,
        Self::Security,
        Self::Review,
        Self::Fix,
        Self::Triage,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Architect => "Architect",
            Self::Pickup => "Pickup",
            Self::Security => "Security",
            Self::Review => "Review",
            Self::Fix => "Fix",
            Self::Triage => "Triage",
        }
    }
}

impl Display for Pass {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} run", self.name())
    }
}

impl serde::Serialize for Pass {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Pass {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|pass| pass.to_string() == name)
            .ok_or_else(|| serde::de::Error::custom("unknown Pass kind"))
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
    crate::github::quota::command::begin();
    static SAMPLER: Once = Once::new();
    SAMPLER.call_once(|| {
        std::thread::spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                drop(record());
            }
        });
    });
    let mut record = record();
    if matches!(begin, Begin::ChildRun(..)) {
        record.inherit_owner(owner::Owner::inherited());
        record.child_of(std::env::var_os(PARENT_RECORD).map(PathBuf::from));
        record.child_dispatch(std::env::var(PARENT_DISPATCH).ok());
    }
    record.begin(begin);
    drop(record);
    attach_quota_budget();
}

fn attach_quota_budget() {
    if let Ok(owner) = captured_owner()
        && let Some(path) = owner.log_path()
        && let Err(error) = crate::github::quota::command::attach(path.into())
    {
        crate::progress::step(format!(
            "warning: could not attach Command quota budget: {error:#}"
        ));
    }
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
/// nothing. Capture the Command's effective waiting-line interval before
/// any waiting stages begin.
pub fn configured(config: &UserConfig) {
    crate::poll::configure_waiting_lines(config.effective_waiting_line_interval());
    record().configured(config);
}

pub(crate) fn preflight_issue(issue: &IssueUrl) {
    record().preflight_issue(issue.clone());
}

/// Keep the Pass's latest start once its repository is known, before gates.
pub(crate) fn pass_fired(repo: &Repo) {
    record().pass_fired(repo);
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
    attach_quota_budget();
}

/// Capture the Custom process's loaded source before recording its start or skip.
pub(crate) fn process_loaded(identity: crate::processes::Identity) {
    record().process_loaded(identity);
}

/// A selected Ticket Process belongs to the active dispatched run.
pub(crate) fn process_running(identity: crate::processes::Identity) {
    record().process_running(identity);
}

pub(crate) fn custom_process() -> Option<crate::processes::Identity> {
    record().custom_process()
}

/// Record that `pass` was skipped on `repo` for `reason`, unless the last
/// Activity log line of its kind there already says so. Nothing is shown:
/// under `activity.quiet_skips`, whatever the pass prints about its skip
/// stays held, and is dropped with the Command log it never kept.
pub fn skipped(pass: Pass, repo: &Repo, reason: impl Display) {
    start_notice::skipped(&reason);
    record().skipped(pass, repo, reason);
}

/// Record a skipped Base branch CI step before the Fix Pass starts a Run.
pub(crate) fn base_ci_skipped(repo: &Repo, reason: &str) {
    record().base_ci_skipped(repo, reason);
}

/// Record that the Custom process `name`, started with no issue, was skipped
/// on `repo` for `reason`. The Activity log keeps `Process <name> skipped:
/// <reason>` unless that is already its last Process line (its own named
/// Process's last skip during a Quota wait). Unlike the other
/// Passes, it also keeps a Command record whose outcome is `skipped`: the
/// Process's record shows that ending. `harness` is the choice it would have
/// run on; the skip does not check that it can run.
pub fn skipped_process(
    repo: &Repo,
    name: &str,
    harness: &crate::harness::Choice,
    reason: &crate::command_limit::NotAdmitted,
) {
    record().skipped_process(repo, name, harness, reason);
}

/// Add an observation to the running command's repository Activity log.
pub(crate) fn activity(message: impl Display) {
    record().activity(message);
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

pub(crate) fn waiting_for(commands: &[String]) {
    record().waiting_for(commands);
}

/// Temporarily overlay the active stage without resetting its timing.
pub(crate) fn quota_wait(until: Option<&str>) {
    if until.is_some() && record().needs_quota_repo() {
        let repo = crate::launch::repo().ok();
        record().preflight_repo(repo);
    }
    record().quota_wait(until);
    if until.is_some() {
        attach_quota_budget();
    }
}

/// Keep the current Session trace, including a Resume's own log, as
/// `session` enters its stage.
pub fn session(session: Logged, path: &Path) {
    record().stage(session.stage(), Some(path));
}

/// What the session just started runs on, for its stage: each Step's
/// actual Harness, Model and Effort, which Plan usage may have routed.
pub fn session_choice(choice: &crate::harness::Choice) {
    record().session_choice(choice);
}

/// `configured` as the Command's implement session ran it: the Harness,
/// Model and Effort its record keeps for that session, which Plan usage may
/// have routed to another route, else `configured` itself. `Built with`
/// names this one tuple.
pub(crate) fn implemented_on(configured: &crate::harness::Choice) -> crate::harness::Choice {
    let recorded = record().implemented_on();
    match recorded.and_then(|(name, model, effort)| {
        Some((crate::harness::Harness::named(&name)?, model, effort))
    }) {
        Some((harness, model, effort)) => crate::harness::Choice {
            harness,
            model,
            effort,
            ..configured.clone()
        },
        None => configured.clone(),
    }
}

/// The selected Issue branch, retained unless completion says it was removed.
pub fn issue_branch(branch: Option<&str>) {
    record().issue_branch(branch);
}

/// The target pull request, available while the Command is still running.
pub(crate) fn pull_request(url: &str) {
    record().pull_request(url);
}

/// The selected Base branch of a Fix Pass's CI repair, before a PR exists.
pub(crate) fn base_branch(branch: &str) {
    record().base_branch(branch);
}

/// Retain Delivery's final counts for the active Run's completion metrics.
pub(crate) fn unaddressed_findings(counts: Option<crate::pull_request::UnaddressedFindings>) {
    record().unaddressed_findings(counts);
}

/// Complete the durable facts from the same account used by other renderers.
pub fn finished(account: &crate::run_ending::Account<'_>) {
    record().finished(account);
}

pub(crate) fn experiment(description: &crate::evals::experiment::Description) {
    record().experiment(description);
}

pub(crate) fn resumable(command: &str) {
    record().resumable(command);
}

/// A Paused Command's Claim put `ready-for-agent` back: a later Pickup run
/// retakes its issue once Plan usage allows.
pub(crate) fn handed_back_for_pickup() {
    record().handed_back_for_pickup();
}

/// Whether this Command's record says a later Pickup run retakes it: its
/// Claim put `ready-for-agent` back when it paused.
pub(crate) fn recorded_handed_back_for_pickup() -> bool {
    record().recorded_handed_back_for_pickup()
}

pub(crate) fn paused(limit: &crate::session::limits::Limit) {
    record().paused(limit);
}

pub(crate) fn experiment_finished(outcome: &str, cause: Option<&str>) {
    complete(outcome, cause);
}

/// Record how a Command with no Run account ended, such as a Process. A
/// Paused Command with no resume command recorded keeps this one.
pub fn complete(outcome: &str, cause: Option<&str>) {
    let mut record = record();
    if outcome == "paused" {
        record.resumable_by_this_command();
    }
    record.outcome(outcome, cause);
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
pub fn dispatching(
    work: Work,
    harness: &Choice,
    security_fix_findings: &[crate::security::RecordedFinding],
) {
    record().dispatching(work, harness, security_fix_findings);
}

/// Print the lines held from the terminal, as before a failure, and print
/// every line from here on.
pub fn show_held() {
    record().show_held();
}

/// The root of `repo`'s logs: `<logs.dir>/<owner>/<repo>/`, named for the
/// GitHub repository, not the checkout, so every checkout of it logs to the
/// same place. Its Command logs, Session traces and Activity log are all under
/// it.
///
/// # Panics
///
/// Before [`configured`], which is a programming error.
pub fn root(repo: &Repo) -> PathBuf {
    record().root(repo)
}

/// [`root`], or nothing before [`configured`].
pub(crate) fn root_if_configured(repo: &Repo) -> Option<PathBuf> {
    record().root_if_configured(repo)
}

/// The command whose folder holds this process's Session traces.
pub fn command_kind() -> CommandKind {
    record().command_kind()
}

/// Resolve a locally retained Run by its pull request, including child Runs.
pub(crate) fn record_for_pull_request(root: &Path, url: &str) -> anyhow::Result<Option<PathBuf>> {
    Ok(run_for_pull_request(root, url)?.map(|(path, _)| path))
}

/// Where this command and all its child Runs keep Session traces on `repo`.
pub fn session_dir(repo: &Repo) -> PathBuf {
    let record = record();
    record
        .root(repo)
        .join("commands")
        .join(record.command_kind().folder())
}

/// The command's start stamp, as in `20261003T120000-0400`, which names its
/// Command log and its Session traces.
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

/// Sessions started while this is held export nothing. `fix-findings` and
/// `close-findings` hold it so a Process command does not send private
/// Security records, or the Ticket runs those Steps start, to OTLP.
static OTLP_SUPPRESSED: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct SuppressOtlp;

impl SuppressOtlp {
    pub(crate) fn new() -> Self {
        OTLP_SUPPRESSED.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for SuppressOtlp {
    fn drop(&mut self) {
        OTLP_SUPPRESSED.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn otlp_suppressed() -> bool {
    OTLP_SUPPRESSED.load(Ordering::Acquire) > 0
}

/// The configured OTLP endpoint, or none for a Security or Scoring command
/// and while a [`SuppressOtlp`] guard is held.
pub(crate) fn otlp_endpoint() -> Option<String> {
    if otlp_suppressed() {
        None
    } else {
        record().otlp_endpoint()
    }
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
    fn process(self) -> crate::processes::Identity {
        use crate::processes::Identity;
        let file = match self {
            Self::Starting { .. } => return Identity::default(),
            Self::Run(_) | Self::BaseFix(_) => "ticket-run",
            Self::SpecRun(_) => "spec-run",
            Self::ArchitectRun(_) => "architect-run",
            Self::SecurityRun(_) => "security-run",
            Self::PickupRun(_) => "pickup-run",
            Self::ScoringRun(_) => "scoring-run",
            Self::ReviewRun(_) => "review-run",
            Self::FixRun(_) => "fix-run",
            Self::TriageRun { .. } => "triage-run",
            Self::Experiment(_) => "experiment",
            Self::Process { name, .. } => return Identity::custom(name, None),
        };
        Identity::built_in(file, &self.kind().to_string())
    }

    fn initial_stage(self) -> Option<Stage> {
        match self {
            Work::Starting { .. } => None,
            Work::Run(_) | Work::BaseFix(_) => Some(Stage::Implement),
            Work::SpecRun(_) => Some(Stage::Tickets),
            Work::ArchitectRun(_) => Some(Stage::ArchitectureReview),
            Work::ScoringRun(_) => Some(Stage::Sampling),
            Work::ReviewRun(_) => Some(Stage::CodeReview),
            Work::FixRun(_) => Some(Stage::PullRequest),
            Work::TriageRun { .. } => Some(Stage::Triage),
            Work::PickupRun(_) | Work::SecurityRun(_) | Work::Process { .. } => None,
            Work::Experiment(_) => Some(Stage::ArmBuilds),
        }
    }

    /// The repository whose work it records.
    fn repo(self) -> Repo {
        match self {
            Work::Starting { repo, .. } => repo.clone(),
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => issue.repo(),
            Work::PickupRun(listed) => listed.issue.repo(),
            Work::ArchitectRun(repo)
            | Work::SecurityRun(repo)
            | Work::ScoringRun(repo)
            | Work::ReviewRun(repo)
            | Work::FixRun(repo)
            | Work::TriageRun { repo, .. }
            | Work::Experiment(repo)
            | Work::Process { repo, .. } => repo.clone(),
        }
    }

    /// Its kind, in the Activity log.
    fn kind(self) -> Kind {
        match self {
            Work::Starting { command, .. } => match command {
                CommandKind::Issue => Kind::Run,
                CommandKind::Architect => Kind::Pass(Pass::Architect),
                CommandKind::Pickup => Kind::Pass(Pass::Pickup),
                CommandKind::Secure => Kind::Pass(Pass::Security),
                CommandKind::Scoring => Kind::ScoringRun,
                CommandKind::Review => Kind::ReviewRun,
                CommandKind::Fix => Kind::FixRun,
                CommandKind::Triage => Kind::TriageRun,
                CommandKind::Experiment => Kind::Experiment,
                CommandKind::Process => Kind::Process,
            },
            Work::Run(_) | Work::BaseFix(_) => Kind::Run,
            Work::SpecRun(_) => Kind::SpecRun,
            Work::PickupRun(_) => Kind::Pass(Pass::Pickup),
            Work::ArchitectRun(_) => Kind::Pass(Pass::Architect),
            Work::SecurityRun(_) => Kind::Pass(Pass::Security),
            Work::ScoringRun(_) => Kind::ScoringRun,
            Work::ReviewRun(_) => Kind::ReviewRun,
            Work::FixRun(_) => Kind::FixRun,
            Work::TriageRun { .. } => Kind::TriageRun,
            Work::Experiment(_) => Kind::Experiment,
            Work::Process { .. } => Kind::Process,
        }
    }

    /// Its issue, if it has one.
    pub(crate) fn issue_url(self) -> Option<&'a IssueUrl> {
        match self {
            Work::Starting { issue, .. } | Work::TriageRun { issue, .. } => issue,
            Work::Run(issue) | Work::BaseFix(issue) | Work::SpecRun(issue) => Some(issue),
            Work::PickupRun(listed) => Some(&listed.issue),
            Work::ArchitectRun(_)
            | Work::SecurityRun(_)
            | Work::ScoringRun(_)
            | Work::ReviewRun(_)
            | Work::FixRun(_)
            | Work::Experiment(_)
            | Work::Process { .. } => None,
        }
    }

    /// Its issue's number, if it has one.
    fn issue(self) -> Option<u64> {
        self.issue_url().map(|issue| issue.number)
    }
}

/// The [`Record`], locked, even if a thread panicked holding it.
fn record() -> MutexGuard<'static, Record<OnMachine>> {
    let mut record = RECORD.lock().unwrap_or_else(PoisonError::into_inner);
    record.check_suspension();
    record
}

/// Cumulative Suspended time measured by this Command. Taking both readings
/// under the recorder's lock also samples the clocks at a Trial's endpoints.
pub(crate) fn suspended_seconds() -> u64 {
    record().suspended_seconds()
}
