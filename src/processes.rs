//! Process commands: inspect Built-in and Custom process files, and run
//! Custom processes as skill, command and catalogue Steps from
//! `~/.thirdshift/processes/<name>.toml`.
//!
//! Steps run in order. Consecutive Steps that name the same `parallel` group
//! run at the same time, each in a throwaway worktree the Process declares.
//! One worktree may be an Issue branch. The Steps that use it are the editing
//! Steps, and a `delivery` Step after the last of them publishes that branch
//! and leaves a pull request ready for review. A `self-merge` Step directly
//! after `delivery` is the Self-merge: it merges when the command's `merge`
//! flag or the User config's `merge_always` asks for one, and otherwise
//! leaves the pull request ready for review. A file can only tighten limits.
//! The catalogue Step `run-published-issue` starts a Ticket run, or a Spec
//! run, on the issue the session Step before it published.
//!
//! A Process started with no issue is a Pass, unless it declares an
//! Issue-branch worktree, which needs an issue. A Pass takes the repository's
//! Pass lock and is skipped, before any worktree, while another Pass holds
//! it. It does not take an Architect run's Ready-issue or Architect-idea
//! gates. A Process started with an issue is not a Pass and does not take the
//! lock.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use toml_edit::{Document, Item, Key, Table};

use crate::args::{ProcessAction, ProcessRun};
use crate::asks::{Asks, Flags};
use crate::base_fix::{BaseFix, BaseFixAsk};
use crate::branch::{self, Selection};
use crate::child_run::ChildAsks;
use crate::claim::{self, Claim};
use crate::config::UserConfig;
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::{self, Asked, Choice, ChosenBy, Harness};
use crate::issue::{IssueInput, IssueUrl};
use crate::logs::{self, Begin, Work};
use crate::notification::{About, NotificationAsk, RunNotification};
use crate::process::{self, Control, Interruption};
use crate::pull_request::PullRequest;
use crate::recovery::{self, Recovery};
use crate::run::Goal;
use crate::run_ending::{Account, Cause};
use crate::worktree::{SkillWorktree, Worktree};
use crate::{delivery, interrupt, launch, progress, prompt, session, skills, trust};

mod catalog;
pub(crate) mod ticket;
use catalog::Catalog;

/// The issue URL when the Process has an issue; unset otherwise.
const ISSUE_URL: &str = "THIRDSHIFT_ISSUE_URL";
/// The folder of Step result files, beside the Command log.
const RESULTS_DIR: &str = "THIRDSHIFT_RESULTS_DIR";
/// The path thirdshift writes this Step's combined output to when it finishes.
const STEP_RESULT: &str = "THIRDSHIFT_STEP_RESULT";

struct ProcessFile {
    description: String,
    worktrees: Vec<WorktreeDraft>,
    steps: Vec<StepDraft>,
    limits: Limits,
    /// Filled by `run` from the merge flag or `merge_always`. Parsing leaves
    /// this ready for review; `check` never merges.
    goal: Goal,
    security_review: bool,
}

/// Limits a file may set. Each one is at least as tight as the built-in rule.
struct Limits {
    /// The Repair cap the Delivery uses. Never above [`crate::delivery::MAX_REPAIRS`].
    repair_cap: usize,
    /// `self_merge = false` forbids a Self-merge the command or User config asked for.
    forbid_self_merge: bool,
    /// Globs. A changed path that matches one holds the Self-merge.
    protected_paths: Vec<String>,
}

impl Limits {
    /// The built-in limits: the effective Repair cap, a merge when asked, and
    /// no protected path.
    fn open() -> Self {
        Self {
            repair_cap: delivery::MAX_REPAIRS,
            forbid_self_merge: false,
            protected_paths: Vec::new(),
        }
    }
}

struct WorktreeDraft {
    name: String,
    /// The `kind` line when this worktree is an Issue branch.
    issue_branch_line: Option<usize>,
}

struct StepDraft {
    name: String,
    name_line: usize,
    optional: bool,
    worktree: Option<(String, usize)>,
    parallel: Option<(String, usize)>,
    kind: DraftKind,
    harness: Option<Harness>,
    /// The Step's `harness` line, or its name when it inherits the Harness.
    report_line: usize,
    model: Option<String>,
    effort: Option<String>,
}

enum DraftKind {
    Skill { skill: String, line: usize },
    Command { command: String },
    Implement { line: usize },
    SecurityReview { line: usize },
    Delivery { line: usize },
    SelfMerge { line: usize },
    RunPublishedIssue { line: usize },
}

struct DeclaredWorktree {
    name: String,
    issue_branch: bool,
}

struct ResolvedStep {
    name: String,
    optional: bool,
    worktree: Option<String>,
    parallel: Option<String>,
    kind: StepKind,
    report_line: usize,
    choice: Choice,
    sources: Sources,
    /// Why this Step's Harness can't start, when preflight failed.
    start_failure: Option<String>,
    /// Startup fallback for an implicitly chosen Security review only.
    review_default: Option<Choice>,
}

impl ResolvedStep {
    fn log_context(&self) -> String {
        let kind = match self.kind {
            StepKind::Skill { .. } => "skill",
            StepKind::Command(_) => "command",
            StepKind::Implement => "implement",
            StepKind::SecurityReview => "security-review",
            StepKind::Delivery => "delivery",
            StepKind::SelfMerge => "self-merge",
            StepKind::RunPublishedIssue => "run-published-issue",
        };
        format!(
            "{kind}; worktree {}; {}",
            self.worktree.as_deref().unwrap_or("throwaway"),
            self.choice
        )
    }

    /// Quote names whose colons could be mistaken for the outcome separator.
    fn log_name(&self) -> String {
        if self.name.contains(": ") || self.name.starts_with('"') {
            serde_json::to_string(&self.name).expect("a Step name is a JSON string")
        } else {
            self.name.clone()
        }
    }

    fn check(&mut self) {
        self.start_failure = self.choice.check().err().map(|error| one_line(&error));
    }

    fn check_security_review(&mut self, enabled: bool, config: &UserConfig) {
        if matches!(self.sources.harness, Source::Step) {
            self.check();
            return;
        }
        if !enabled {
            return;
        }
        if let Some(security) = Choice::of_security_review(config.security_harness, &config.harness)
        {
            self.choice.harness = security.harness;
            self.choice.chosen_by = security.chosen_by;
            self.sources.harness = Source::UserConfig;
            if !matches!(self.sources.model, Source::Step) {
                self.sources.model = Source::word(&None, &security.model);
                self.choice.model = security.model;
            }
            if !matches!(self.sources.effort, Source::Step) {
                self.sources.effort = Source::word(&None, &security.effort);
                self.choice.effort = security.effort;
            }
        }
        let mut default = Choice::of(&Asked::default(), &config.harness);
        self.start_failure = self
            .choice
            .check_security_review(&mut default)
            .err()
            .map(|error| one_line(&error));
        self.review_default = Some(default);
    }
}

/// Where a Step's Harness, Model and Effort each came from.
#[derive(Clone, Copy)]
struct Sources {
    harness: Source,
    model: Source,
    effort: Source,
}

/// The Command's resolved choice and its origins, inherited together by Steps.
struct CommandChoice {
    choice: Choice,
    sources: Sources,
}

impl CommandChoice {
    fn of(asked: &Asked, settings: &harness::Settings) -> Self {
        let choice = Choice::of(asked, settings);
        let sources = Sources {
            harness: source_from(choice.chosen_by),
            model: Source::word(&asked.model_and_effort.model, &choice.model),
            effort: Source::word(&asked.model_and_effort.effort, &choice.effort),
        };
        Self { choice, sources }
    }

    /// Settle catalog aliases when the Command's Harness is actually used,
    /// then check each Step's effective choice with the usual preflight.
    fn check_steps(&mut self, steps: &mut [ResolvedStep]) {
        if steps
            .iter()
            .any(|step| step.choice.harness == self.choice.harness)
        {
            let mut settled = self.choice.clone();
            // A Step may replace a rejected Command field. Only its effective
            // choice can fail the Process; this pass just resolves aliases.
            if settled.check_without_probe().is_ok() {
                self.choice = settled;
            }
        }
        for step in steps {
            if !matches!(step.sources.model, Source::Step) {
                step.choice.model = self.choice.model.clone();
            }
            if !matches!(step.sources.effort, Source::Step) {
                step.choice.effort = self.choice.effort.clone();
            }
            if !matches!(step.kind, StepKind::SecurityReview) {
                step.check();
            }
        }
    }

    /// Only fields explicitly named by the Step replace the Command's values.
    fn step(&self, step: &StepDraft) -> (Choice, Sources) {
        let mut choice = self.choice.clone();
        let mut sources = self.sources;
        if let Some(harness) = step.harness {
            choice.harness = harness;
            choice.chosen_by = ChosenBy::Step;
            sources.harness = Source::Step;
        }
        if let Some(model) = &step.model {
            choice.model = Some(model.clone());
            sources.model = Source::Step;
        }
        if let Some(effort) = &step.effort {
            choice.effort = Some(effort.clone());
            sources.effort = Source::Step;
        }
        (choice, sources)
    }
}

#[derive(Clone, Copy)]
enum Source {
    Step,
    Command,
    UserConfig,
    Default,
}

impl Source {
    fn word(asked: &Option<String>, chosen: &Option<String>) -> Self {
        if asked.is_some() {
            Self::Command
        } else if chosen.is_some() {
            Self::UserConfig
        } else {
            Self::Default
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Source::Step => "Step",
            Source::Command => "command",
            Source::UserConfig => "User config",
            Source::Default => "default",
        })
    }
}

fn source_from(chosen_by: ChosenBy) -> Source {
    match chosen_by {
        ChosenBy::Command => Source::Command,
        ChosenBy::UserConfig | ChosenBy::SecurityConfig => Source::UserConfig,
        ChosenBy::Default => Source::Default,
        ChosenBy::Step => Source::Step,
    }
}

enum StepKind {
    Skill { name: String, path: PathBuf },
    Command(String),
    Implement,
    SecurityReview,
    Delivery,
    SelfMerge,
    RunPublishedIssue,
}

struct OwnedWorktree {
    name: Option<String>,
    checkout: SkillWorktree,
}

struct Prepared {
    path: PathBuf,
    worktrees: Vec<OwnedWorktree>,
    declared: Vec<DeclaredWorktree>,
    /// The Issue-branch worktree's name, when the file declares one.
    issue_worktree: Option<String>,
    _written: WrittenGuard,
    choice: Choice,
    description: String,
    steps: Vec<ResolvedStep>,
    issue: Option<IssueUrl>,
    /// When untrusted edits start counting, set when a run begins.
    started: DateTime<Utc>,
    /// The Base branch checked out in the Launch directory.
    base: String,
    limits: Limits,
    /// `Merged` only when this run was asked to Self-merge.
    goal: Goal,
    security_review: bool,
    completed: Mutex<CompletedRuns>,
    /// Killed work retained across session Steps, ordered by Step even when
    /// a parallel group completes out of order.
    killed_work: Mutex<BTreeMap<usize, Vec<String>>>,
}

/// Actual published outcomes, including dispatched Runs in parallel groups.
#[derive(Default)]
struct CompletedRuns {
    resolutions: Vec<crate::run::Resolution>,
    failed: bool,
    /// Only this Process's implement Step can end the Process early.
    unreproduced_bug: Option<String>,
}

/// The Issue branch a run is delivering, beside the throwaway worktrees.
struct IssueRun<'a> {
    checkout: &'a mut Option<Worktree>,
    issue: &'a IssueUrl,
    branch: &'a str,
    base: &'a str,
    on_origin: bool,
    selection: &'a Selection,
    logs: &'a session::Logs,
    base_fix: &'a mut BaseFix,
    process: &'a str,
    /// Delivery left a pull request ready for review, or merged it.
    delivered: bool,
    pull_request: PullRequest,
    reviews: Vec<crate::security::review::Outcome>,
}

/// Ends the Claim on drop unless the run removed it or ended it already.
struct HeldClaim<'a> {
    claim: Option<Claim<'a>>,
}

impl HeldClaim<'_> {
    fn needs_info(&mut self) {
        if let Some(claim) = self.claim.take() {
            claim.needs_info();
        }
    }

    fn remove(&mut self) {
        if let Some(claim) = self.claim.take() {
            claim.remove();
        }
    }

    fn release(&mut self) {
        if let Some(claim) = self.claim.take() {
            claim.end(None);
        }
    }
}

impl Drop for HeldClaim<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

/// What one Step needs from the Process, shared by a parallel group.
struct StepRun<'a> {
    process: &'a str,
    goal: Goal,
    limits: &'a Limits,
    completed: &'a Mutex<CompletedRuns>,
    killed_work: &'a Mutex<BTreeMap<usize, Vec<String>>>,
    issue_branch: bool,
    choice: &'a Choice,
    issue: Option<&'a IssueUrl>,
    started: DateTime<Utc>,
    /// Branch and Base branch when this Step continues an Issue branch.
    continuation: Option<(String, String)>,
    checkout: Option<&'a Worktree>,
    selection: Option<&'a Selection>,
    /// The Base branch checked out in the Launch directory.
    base: &'a str,
    steps: &'a [ResolvedStep],
}

/// Results and start times of Steps that have already finished.
struct PriorSteps<'a> {
    results: &'a [(String, PathBuf)],
    began: &'a [(String, DateTime<Utc>)],
}

struct WrittenGuard;

impl Drop for WrittenGuard {
    fn drop(&mut self) {
        skills::remove_written();
    }
}

pub fn execute(action: ProcessAction) -> Result<String> {
    match action {
        ProcessAction::List => Catalog::installed()?.list(),
        ProcessAction::Show(name) => Catalog::installed()?.show(&name),
        ProcessAction::Check(command) => check(&command),
        ProcessAction::Run(command) => run(&command),
    }
}

fn check(command: &ProcessRun) -> Result<String> {
    let name = &command.name;
    interrupt::install()?;
    let config = UserConfig::load()?;
    let (path, text) = Catalog::installed()?.source(name)?;
    let mut file = validated(&path, &text)?;
    file.security_review = config.security_review;
    let issue = resolve_process_issue(command.issue.clone())?;
    require_issue(&path, &file, issue.as_ref())?;
    let base = launch::checked_out_base()?;
    let launch = Git::new(std::env::current_dir()?);
    let prepared = prepare_from(
        &path,
        &config,
        &command.asked,
        file,
        issue,
        Utc::now(),
        WorktreeSource {
            git: &launch,
            commit: base.origin_commit(),
            issue_checkout: None,
            base: base.name(),
        },
    )?;
    let text = plan(name, &prepared);
    if let Err(error) = refuse_unstartable(&prepared) {
        println!("{text}");
        return Err(error);
    }
    Ok(text)
}

fn run(command: &ProcessRun) -> Result<String> {
    let name = &command.name;
    interrupt::install()?;
    let config = UserConfig::load()?;
    // The same check as a Ticket run, before any worktree or Step: a missing
    // address or Resend key fails the Process before it starts. As for the
    // other Passes, the check is before a skip, so a Process that would be
    // skipped fails on it too.
    let goal = Goal::from_flag(command.goal, config.merge_always);
    let notification = RunNotification::new(
        Flags {
            email: command.email.clone(),
            ..Flags::default()
        }
        .notification(&config),
        &config.email,
        About::Process { name },
    )?;
    let (path, mut file) = match load(name) {
        Ok(loaded) => loaded,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    file.goal = goal;
    file.security_review = Flags {
        security_review: command.security_review,
        ..Flags::default()
    }
    .security_review(&config);
    // Before any worktree: a foreign Issue URL is an origin mismatch, and a
    // number that cannot be resolved stops the same way. An Issue-branch
    // worktree with no issue is refused before the Pass log. A failure here
    // is still a failed Process, so it sends the Run notification a Ticket
    // run would send for the same kind of stop.
    let issue = match resolve_process_issue(command.issue.clone()) {
        Ok(issue) => issue,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    if let Err(error) = require_issue(&path, &file, issue.as_ref()) {
        return Err(failed_before_steps(notification, error));
    }
    // No issue: a Pass. The Base branch is checked before the lock, so a bad
    // checkout fails instead of being skipped, and a skip creates no worktree.
    // The lock is then held until this process exits, through every Step.
    // `pass` holds the first lines, so a quiet skip prints nothing. A skip
    // sends no Run notification.
    if issue.is_none() {
        logs::begin(Begin::Process { name, pass: true });
        logs::configured(&config);
        let base = match launch::checked_out_base() {
            Ok(base) => base,
            Err(error) => return Err(failed_before_steps(notification, error)),
        };
        let repo = match launch::repo() {
            Ok(repo) => repo,
            Err(error) => return Err(failed_before_steps(notification, error)),
        };
        match launch::hold_pass(&repo) {
            Ok(Some(running)) => {
                let reason = running.to_string();
                let choice = Choice::of(&command.asked, &config.harness);
                progress::step(&reason);
                logs::skipped_process(&repo, name, &choice, &reason);
                return Ok(String::new());
            }
            Ok(None) => {}
            Err(error) => return Err(failed_before_steps(notification, error)),
        }
        let started = Utc::now() - crate::run::STARTED_MARGIN;
        let launch = match std::env::current_dir() {
            Ok(dir) => Git::new(dir),
            Err(error) => return Err(failed_before_steps(notification, error.into())),
        };
        let prepared = match prepare_from(
            &path,
            &config,
            &command.asked,
            file,
            None,
            started,
            WorktreeSource {
                git: &launch,
                commit: base.origin_commit(),
                issue_checkout: None,
                base: base.name(),
            },
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Err(failed_before_steps(notification, error)),
        };
        return finish_run(name, &repo, prepared, notification);
    }
    let issue = issue.expect("an issue");
    let started = Utc::now() - crate::run::STARTED_MARGIN;
    if file_has_issue_branch(&file) {
        return run_issue_branch(command, &config, &path, file, issue, started, notification);
    }
    let base = match launch::checked_out_base() {
        Ok(base) => base,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let launch = match std::env::current_dir() {
        Ok(dir) => Git::new(dir),
        Err(error) => return Err(failed_before_steps(notification, error.into())),
    };
    let prepared = match prepare_from(
        &path,
        &config,
        &command.asked,
        file,
        Some(issue),
        started,
        WorktreeSource {
            git: &launch,
            commit: base.origin_commit(),
            issue_checkout: None,
            base: base.name(),
        },
    ) {
        Ok(prepared) => prepared,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let repo = match launch::repo() {
        Ok(repo) => repo,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    logs::begin(Begin::Process { name, pass: false });
    logs::configured(&config);
    finish_run(name, &repo, prepared, notification)
}

/// A stop after the email check and before any Step is still a failed Process.
fn failed_before_steps(notification: RunNotification, error: anyhow::Error) -> anyhow::Error {
    let cause = format!("{error:#}");
    notify(
        notification,
        "failed",
        Some(&cause),
        &CompletedRuns::default(),
    );
    error
}

/// Save and, when asked, send the Run notification. A skipped Process sends none.
fn notify(
    notification: RunNotification,
    outcome: &'static str,
    cause: Option<&str>,
    completed: &CompletedRuns,
) {
    if outcome == "skipped" {
        return;
    }
    let merged = outcome == "succeeded"
        && !completed.failed
        && !completed.resolutions.is_empty()
        && completed
            .resolutions
            .iter()
            .all(|resolution| resolution.goal() == Some(Goal::Merged));
    let not_reproduced = completed.unreproduced_bug.as_ref();
    let account_outcome = if outcome == "succeeded" && not_reproduced.is_some() {
        "bug not reproduced"
    } else if merged {
        Goal::Merged.outcome()
    } else {
        outcome
    };
    let ended = if outcome == "succeeded" {
        Ok(match not_reproduced {
            Some(url) => format!("bug not reproduced: {url}"),
            None => account_outcome.to_string(),
        })
    } else {
        Err(Cause::of(&anyhow!("{}", cause.unwrap_or(outcome))))
    };
    let account = Account {
        recovery: None,
        outcome: account_outcome,
        ended,
        interrupted: outcome == "interrupted",
        pr_url: completed
            .resolutions
            .iter()
            .rev()
            .find_map(|resolution| match resolution {
                crate::run::Resolution::PullRequest { url, .. } => Some(url.as_str()),
                _ => None,
            }),
        advice: &[],
        base_fix: None,
        log: None,
        ticket_lines: &[],
        review: None,
        failed_reproduction: None,
        pre_audit_failure: None,
        security_findings: None,
        security_findings_waiting: false,
        security_fix: None,
        security_close: None,
        urls: completed
            .resolutions
            .iter()
            .map(|resolution| resolution.url())
            .collect(),
    };
    notification.finish(&account);
}

fn finish_run(
    name: &str,
    repo: &crate::issue::Repo,
    prepared: Prepared,
    mut notification: RunNotification,
) -> Result<String> {
    if let Err(error) = refuse_unstartable(&prepared) {
        return Err(failed_before_steps(notification, error));
    }
    logs::started(Work::Process { repo, name }, &prepared.choice);
    notification.built_with(&prepared.choice);
    let result = execute_steps(name, &prepared, None);
    let (outcome, cause) = classify(&result);
    let completed = std::mem::take(&mut *prepared.completed.lock().expect("completed Runs"));
    drop(prepared);
    logs::ended(outcome);
    logs::complete(outcome, cause.as_deref());
    notify(notification, outcome, cause.as_deref(), &completed);
    result
}

fn run_issue_branch(
    command: &ProcessRun,
    config: &UserConfig,
    path: &Path,
    file: ProcessFile,
    issue: IssueUrl,
    started: DateTime<Utc>,
    mut notification: RunNotification,
) -> Result<String> {
    let name = &command.name;
    let base_branch = match launch::checked_out_base() {
        Ok(base) => base,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let launch_git = match std::env::current_dir() {
        Ok(dir) => Git::new(dir),
        Err(error) => return Err(failed_before_steps(notification, error.into())),
    };
    let selection = match branch::select(&launch_git, &GitHub::new(), &issue) {
        Ok(selection) => selection,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let base = selection
        .pr_base(None, Some(base_branch.name()), |line| progress::step(&line))
        .unwrap_or(base_branch.name())
        .to_string();
    let on_origin = matches!(selection, Selection::Continuation { .. });
    let branch = selection.branch().to_string();
    let made = match claim::make(&issue, &launch_git, &branch) {
        Ok(claim) => claim,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let mut claim = HeldClaim { claim: Some(made) };
    let checkout = match &selection {
        Selection::Fresh { .. } => Worktree::create_fresh(&launch_git, &issue.repo, &branch, &base),
        Selection::Continuation { .. } => {
            Worktree::continue_existing(&launch_git, &issue.repo, &branch, &base)
        }
    };
    let checkout = match checkout {
        Ok(checkout) => checkout,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let commit = if base == base_branch.name() {
        base_branch.origin_commit().to_string()
    } else {
        match launch_git.run(&["rev-parse", &format!("origin/{base}")]) {
            Ok(commit) => commit,
            Err(error) => return Err(failed_before_steps(notification, error)),
        }
    };
    let prepared = match prepare_from(
        path,
        config,
        &command.asked,
        file,
        Some(issue.clone()),
        started,
        WorktreeSource {
            git: &launch_git,
            commit: &commit,
            issue_checkout: Some(checkout.path()),
            base: &base,
        },
    ) {
        Ok(prepared) => prepared,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    if let Err(error) = refuse_unstartable(&prepared) {
        return Err(failed_before_steps(notification, error));
    }
    let repo = match launch::repo() {
        Ok(repo) => repo,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    let mut held = Some(checkout);
    // Session traces ask for the log root, which exists once this command is
    // configured. The Command record stays the Process; the traces are named
    // for the issue.
    logs::begin(Begin::Process { name, pass: false });
    logs::configured(config);
    let logs = session::Logs::of_run(&issue);
    let mut base_fix = BaseFix::new(None, BaseFixAsk::Forbid);
    logs::started(Work::Process { repo: &repo, name }, &prepared.choice);
    notification.built_with(&prepared.choice);
    let delivered;
    let result = {
        let mut issue_run = IssueRun {
            checkout: &mut held,
            issue: &issue,
            branch: &branch,
            base: &base,
            on_origin,
            selection: &selection,
            logs: &logs,
            base_fix: &mut base_fix,
            process: name,
            delivered: false,
            pull_request: PullRequest::new(&issue, &branch, &base, GitHub::new()),
            reviews: Vec::new(),
        };
        let result = execute_steps(name, &prepared, Some(&mut issue_run));
        let context = continuation(&issue_run);
        let kept_on_origin = issue_run.on_origin;
        if let Err(error) = &result
            && let Some(checkout) = issue_run.checkout.as_mut()
        {
            preserve_step_failure(checkout, context, kept_on_origin, error);
        }
        delivered = issue_run.delivered;
        result
    };
    let not_reproduced = prepared
        .completed
        .lock()
        .expect("completed Runs")
        .unreproduced_bug
        .is_some();
    if result.is_ok() && not_reproduced {
        claim.needs_info();
    } else if result.is_ok() || delivered {
        claim.remove();
    } else {
        claim.release();
    }
    let (outcome, cause) = classify(&result);
    let completed = std::mem::take(&mut *prepared.completed.lock().expect("completed Runs"));
    drop(prepared);
    drop(held);
    logs::ended(outcome);
    if let Ok(comment_url) = &result
        && not_reproduced
    {
        logs::complete(
            "bug not reproduced",
            Some(&format!("bug not reproduced: {comment_url}")),
        );
    } else {
        logs::complete(outcome, cause.as_deref());
    }
    notify(notification, outcome, cause.as_deref(), &completed);
    result
}

/// Link every distinct Harness's Factory skills before any session starts.
fn link_harnesses(worktree: &Path, steps: &[ResolvedStep]) -> Result<()> {
    let mut linked = Vec::new();
    for step in steps {
        if linked.contains(&step.choice.harness) {
            continue;
        }
        linked.push(step.choice.harness);
        let adapter = step.choice.harness.adapter();
        skills::link_into(worktree, adapter)?;
        adapter.link_instruction_fallback(worktree)?;
    }
    Ok(())
}

/// The first Step whose Harness can't start, before any session runs.
fn refuse_unstartable(prepared: &Prepared) -> Result<()> {
    let Some(step) = prepared
        .steps
        .iter()
        .find(|step| step.start_failure.is_some())
    else {
        return Ok(());
    };
    let reason = step.start_failure.as_deref().unwrap_or_default();
    Err(rule(
        &prepared.path,
        step.report_line,
        "harness",
        format!(
            "step {}'s Harness {} can't start: {reason}",
            step.name,
            step.choice.harness.name()
        ),
    ))
}

fn execute_steps(
    process: &str,
    prepared: &Prepared,
    issue_run: Option<&mut IssueRun<'_>>,
) -> Result<String> {
    let result = execute_groups(process, prepared, issue_run);
    let endings: Vec<_> = prepared
        .killed_work
        .lock()
        .expect("killed work")
        .values()
        .flatten()
        .cloned()
        .collect();
    session::with_killed_work(result, &endings)
}

fn execute_groups(
    process: &str,
    prepared: &Prepared,
    mut issue_run: Option<&mut IssueRun<'_>>,
) -> Result<String> {
    for worktree in &prepared.worktrees {
        link_harnesses(worktree.checkout.path(), &prepared.steps)?;
    }
    if let Some(checkout) = issue_run.as_ref().and_then(|run| run.checkout.as_ref()) {
        link_harnesses(checkout.path(), &prepared.steps)?;
    }
    let results = logs::command_log_path()
        .context("a Process has no Command log")?
        .with_extension("steps");
    fs::create_dir_all(&results)
        .with_context(|| format!("could not create {}", results.display()))?;
    let mut earlier = Vec::new();
    let mut began = Vec::new();
    let mut last = String::new();
    let ranges = group_ranges(&prepared.steps);
    let mut index = 0;
    while index < ranges.len() {
        let range = ranges[index].clone();
        let group = &prepared.steps[range.clone()];
        if group.len() == 1 && matches!(group[0].kind, StepKind::Delivery) {
            let merge_at = ranges.get(index + 1).and_then(|next| {
                (next.len() == 1 && matches!(prepared.steps[next.start].kind, StepKind::SelfMerge))
                    .then_some(next.start)
            });
            let Some(issue_run) = issue_run.as_mut() else {
                bail!("delivery has no Issue-branch worktree");
            };
            let started = Instant::now();
            let began_at = Utc::now();
            let (delivery, merge) =
                run_delivery(prepared, &group[0], issue_run, merge_at.is_some());
            finish_step(
                &group[0],
                started.elapsed(),
                (began_at, delivery),
                &results,
                &mut earlier,
                &mut began,
                &mut last,
            )
            .map_err(|error| skip_remaining(&prepared.steps[range.end..], &group[0].name, error))?;
            if let Some(step_index) = merge_at {
                let started = Instant::now();
                let began_at = Utc::now();
                finish_step(
                    &prepared.steps[step_index],
                    started.elapsed(),
                    (
                        began_at,
                        merge.expect("a following self-merge has a result"),
                    ),
                    &results,
                    &mut earlier,
                    &mut began,
                    &mut last,
                )
                .map_err(|error| {
                    skip_remaining(
                        &prepared.steps[step_index + 1..],
                        &prepared.steps[step_index].name,
                        error,
                    )
                })?;
                index += 1;
            }
            index += 1;
            continue;
        }
        let prior = PriorSteps {
            results: &earlier,
            began: &began,
        };
        let outcomes = if group.len() == 1 {
            let started = Instant::now();
            let began_at = Utc::now();
            let outcome = if matches!(group[0].kind, StepKind::SecurityReview) {
                run_security_review(
                    prepared,
                    &group[0],
                    range.start,
                    issue_run
                        .as_deref_mut()
                        .context("security-review has no Issue branch")?,
                )
            } else {
                run_step(
                    process,
                    prepared,
                    &group[0],
                    &prior,
                    &results,
                    range.start == 0,
                    issue_run.as_deref(),
                )
            };
            vec![(started.elapsed(), began_at, outcome)]
        } else {
            run_parallel(
                process,
                prepared,
                group,
                &prior,
                &results,
                range.start,
                issue_run.as_deref(),
            )
        };
        let mut group_error = None;
        for (step, (elapsed, began_at, outcome)) in group.iter().zip(outcomes) {
            match finish_step(
                step,
                elapsed,
                (began_at, outcome),
                &results,
                &mut earlier,
                &mut began,
                &mut last,
            ) {
                Ok(()) => {}
                Err(error) if group_error.is_none() => group_error = Some((&step.name, error)),
                Err(_) => {}
            }
        }
        if let Some((name, error)) = group_error {
            return Err(skip_remaining(&prepared.steps[range.end..], name, error));
        }
        let completed = prepared.completed.lock().expect("completed Runs");
        if let Some(comment_url) = &completed.unreproduced_bug {
            let message = format!("bug not reproduced: {comment_url}");
            progress::step(&message);
            for step in &prepared.steps[range.end..] {
                progress::step(format_args!(
                    "step {}: 0s skipped: {message} ({})",
                    step.log_name(),
                    step.log_context()
                ));
            }
            return Ok(comment_url.clone());
        }
        index += 1;
    }
    Ok(last)
}

/// Account for Steps that cannot start after a required Step failed.
fn skip_remaining(steps: &[ResolvedStep], failed: &str, error: anyhow::Error) -> anyhow::Error {
    for step in steps {
        progress::step(format_args!(
            "step {}: 0s skipped: Step {failed} did not complete: {} ({})",
            step.log_name(),
            one_line(&error),
            step.log_context(),
        ));
    }
    error
}

/// Consecutive items that share a group name. An item with no group, and a
/// repeat of a name after an item outside it, each start their own range.
fn consecutive_groups<T>(
    items: &[T],
    name_of: impl Fn(&T) -> Option<&str>,
) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < items.len() {
        let start = index;
        let group = name_of(&items[index]).map(str::to_string);
        index += 1;
        if let Some(group) = group {
            while index < items.len() && name_of(&items[index]) == Some(group.as_str()) {
                index += 1;
            }
        }
        ranges.push(start..index);
    }
    ranges
}

fn group_ranges(steps: &[ResolvedStep]) -> Vec<std::ops::Range<usize>> {
    consecutive_groups(steps, |step| step.parallel.as_deref())
}

fn run_parallel(
    process: &str,
    prepared: &Prepared,
    group: &[ResolvedStep],
    prior: &PriorSteps<'_>,
    results: &Path,
    group_start: usize,
    issue_run: Option<&IssueRun<'_>>,
) -> Vec<(Duration, DateTime<Utc>, Result<String>)> {
    let (checkout, continuing) = held_issue_branch(issue_run);
    let steps = &prepared.steps;
    let base = prepared.base.as_str();
    let goal = prepared.goal;
    let limits = &prepared.limits;
    let completed = &prepared.completed;
    let killed_work = &prepared.killed_work;
    let selection = issue_run.map(|run| run.selection);
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(group.len());
        for (offset, step) in group.iter().enumerate() {
            let cwd = step_dir(prepared, step, checkout).to_path_buf();
            let choice = step.choice.clone();
            let earlier = prior.results.to_vec();
            let began = prior.began.to_vec();
            let process = process.to_string();
            let results = results.to_path_buf();
            let first = group_start + offset == 0;
            let issue = prepared.issue.clone();
            let started_at = prepared.started;
            let continuation = step_continuation(prepared, step, continuing.as_ref());
            let issue_branch =
                names_issue_worktree(step.worktree.as_deref(), prepared.issue_worktree.as_deref());
            handles.push(scope.spawn(move || {
                let started = Instant::now();
                let began_at = Utc::now();
                let prior = PriorSteps {
                    results: &earlier,
                    began: &began,
                };
                let run = StepRun {
                    process: &process,
                    goal,
                    limits,
                    completed,
                    killed_work,
                    issue_branch,
                    choice: &choice,
                    issue: issue.as_ref(),
                    started: started_at,
                    continuation,
                    checkout,
                    selection,
                    base,
                    steps,
                };
                let outcome = run_step_in(&run, &cwd, step, &prior, &results, first);
                (started.elapsed(), began_at, outcome)
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("a step thread panicked"))
            .collect()
    })
}

fn finish_step(
    step: &ResolvedStep,
    elapsed: Duration,
    outcome: (DateTime<Utc>, Result<String>),
    results: &Path,
    earlier: &mut Vec<(String, PathBuf)>,
    began: &mut Vec<(String, DateTime<Utc>)>,
    last: &mut String,
) -> Result<()> {
    let (began_at, outcome) = outcome;
    let duration = progress::duration(elapsed);
    match outcome {
        Ok(message) => {
            progress::step(format_args!(
                "step {}: {duration} succeeded ({})",
                step.log_name(),
                step.log_context()
            ));
            let path = results.join(format!("{}.txt", step.name));
            fs::write(&path, &message)
                .with_context(|| format!("could not write {}", path.display()))?;
            earlier.push((step.name.clone(), path));
            began.push((step.name.clone(), began_at));
            *last = message;
            Ok(())
        }
        Err(error) => {
            progress::step(format_args!(
                "step {}: {duration} {}: {} ({})",
                step.log_name(),
                failure_outcome(),
                one_line(&error),
                step.log_context()
            ));
            // A file cannot loosen a catalogue review's trust gate. Review
            // failures are retained as holds; an untrusted edit stops here.
            if step.optional
                && !matches!(step.kind, StepKind::SecurityReview)
                && !interrupt::requested()
            {
                let path = results.join(format!("{}.txt", step.name));
                if path.is_file() {
                    *last = fs::read_to_string(&path)
                        .with_context(|| format!("could not read {}", path.display()))?;
                    earlier.push((step.name.clone(), path));
                    began.push((step.name.clone(), began_at));
                }
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

fn run_step(
    process: &str,
    prepared: &Prepared,
    step: &ResolvedStep,
    prior: &PriorSteps<'_>,
    results: &Path,
    first: bool,
    issue_run: Option<&IssueRun<'_>>,
) -> Result<String> {
    let (checkout, continuing) = held_issue_branch(issue_run);
    let run = StepRun {
        process,
        goal: prepared.goal,
        limits: &prepared.limits,
        completed: &prepared.completed,
        killed_work: &prepared.killed_work,
        issue_branch: names_issue_worktree(
            step.worktree.as_deref(),
            prepared.issue_worktree.as_deref(),
        ),
        choice: &step.choice,
        issue: prepared.issue.as_ref(),
        started: prepared.started,
        continuation: step_continuation(prepared, step, continuing.as_ref()),
        checkout,
        selection: issue_run.map(|run| run.selection),
        base: &prepared.base,
        steps: &prepared.steps,
    };
    run_step_in(
        &run,
        step_dir(prepared, step, checkout),
        step,
        prior,
        results,
        first,
    )
}

fn step_dir<'a>(
    prepared: &'a Prepared,
    step: &ResolvedStep,
    checkout: Option<&'a Worktree>,
) -> &'a Path {
    if let Some(checkout) = checkout
        && names_issue_worktree(step.worktree.as_deref(), prepared.issue_worktree.as_deref())
    {
        return checkout.path();
    }
    worktree_path(&prepared.worktrees, step.worktree.as_deref())
}

/// The Issue-branch checkout a run holds, and the branch names when that
/// branch already has commits on origin.
fn held_issue_branch<'a>(
    issue_run: Option<&'a IssueRun<'_>>,
) -> (Option<&'a Worktree>, Option<(String, String)>) {
    let Some(run) = issue_run else {
        return (None, None);
    };
    let continuing = run
        .on_origin
        .then(|| (run.branch.to_string(), run.base.to_string()));
    (run.checkout.as_ref(), continuing)
}

/// Branch and Base branch for a Step whose checkout continues the Issue branch.
fn step_continuation(
    prepared: &Prepared,
    step: &ResolvedStep,
    continuing: Option<&(String, String)>,
) -> Option<(String, String)> {
    let (branch, base) = continuing?;
    names_issue_worktree(step.worktree.as_deref(), prepared.issue_worktree.as_deref())
        .then(|| (branch.clone(), base.clone()))
}

fn names_issue_worktree(step: Option<&str>, issue: Option<&str>) -> bool {
    matches!((step, issue), (Some(step), Some(issue)) if step == issue)
}

fn run_security_review(
    prepared: &Prepared,
    step: &ResolvedStep,
    index: usize,
    issue_run: &mut IssueRun<'_>,
) -> Result<String> {
    if !prepared.security_review {
        let skipped = "Security review skipped: not requested";
        progress::step(skipped);
        return Ok(skipped.to_string());
    }
    let checkout = issue_run
        .checkout
        .as_ref()
        .context("the Issue-branch worktree is gone")?;
    let before_each = || prepare_for_session(issue_run.issue, prepared.started, checkout.path());
    let text = prompt::security_review(issue_run.issue, issue_run.base, issue_run.branch);
    let logs = session::Logs::of_process_review(&issue_run.issue.repo(), index + 1);
    let (review, log) = session::Sessions::checked_within(
        &logs,
        checkout.path(),
        &step.choice,
        None,
        step.review_default.as_ref(),
        &before_each,
        |sessions| {
            let review = crate::security::review::step::Step {
                sessions,
                worktree: checkout,
                issue: issue_run.issue,
                base: issue_run.base,
            }
            .run(&mut issue_run.pull_request, &text);
            retain_killed_work(&prepared.killed_work, index + 1, sessions);
            review
        },
    );
    let review = match review {
        Ok(review) => review,
        Err(error) if error.is::<delivery::UntrustedEdit>() => return Err(error),
        Err(error) => crate::security::review::Outcome::of(Err(error), log),
    };
    let entry = review.entry();
    issue_run.reviews.push(review);
    issue_run.pull_request.security_reviewed(
        issue_run
            .reviews
            .iter()
            .map(crate::security::review::Outcome::unaddressed_count)
            .sum(),
    );
    Ok(entry)
}

/// Publish the Issue branch, and when `merge_follows` run the Self-merge in
/// that same Delivery. The Self-merge is the existing one: one Repair loop,
/// then the merge when the run's goal asks for it and the file does not forbid it.
/// A protected path, `self_merge = false`, or a policy refusal leaves the
/// pull request ready and fails the self-merge Step. The pair's results are
/// the delivery Step's and, when a self-merge Step follows, that Step's.
fn run_delivery(
    prepared: &Prepared,
    step: &ResolvedStep,
    issue_run: &mut IssueRun<'_>,
    merge_follows: bool,
) -> (Result<String>, Option<Result<String>>) {
    let checkout = match issue_run.checkout.take() {
        Some(checkout) => checkout,
        None => return (Err(anyhow!("the Issue-branch worktree is gone")), None),
    };
    let asked = prepared.goal == Goal::Merged;
    let forbid = prepared.limits.forbid_self_merge;
    let merge = merge_follows && asked && !forbid;
    progress::step(format_args!(
        "running step {} with {choice}",
        step.name,
        choice = step.choice
    ));
    if merge {
        let merge_step = prepared
            .steps
            .iter()
            .find(|step| matches!(step.kind, StepKind::SelfMerge));
        if let Some(merge_step) = merge_step {
            progress::step(format_args!("running step {}", merge_step.name));
        }
    }
    let child_asks = ChildAsks {
        harness: step.choice.clone(),
        ..ChildAsks::default()
    };
    let started = prepared.started;
    let protected = if merge {
        prepared.limits.protected_paths.as_slice()
    } else {
        &[]
    };
    let published = (delivery::Delivery {
        security_review: false,
        child_asks: &child_asks,
        issue: issue_run.issue,
        base: issue_run.base,
        goal: if merge {
            Goal::Merged
        } else {
            Goal::ReadyForReview
        },
        base_fix: issue_run.base_fix,
        logs: issue_run.logs,
        started,
        repair_cap: prepared.limits.repair_cap,
        protected_paths: protected,
    })
    .publish(checkout, &mut issue_run.pull_request, &issue_run.reviews);
    match published {
        Ok(url) => {
            issue_run.delivered = true;
            prepared
                .completed
                .lock()
                .expect("completed Runs")
                .resolutions
                .push(crate::run::Resolution::PullRequest {
                    url: url.clone(),
                    goal: if merge {
                        Goal::Merged
                    } else {
                        Goal::ReadyForReview
                    },
                });
            let merge_result = if !merge_follows {
                None
            } else if asked && forbid {
                Some(Err(anyhow!(
                    "self_merge = false holds the Self-merge; the pull request stays ready for review"
                )))
            } else {
                Some(Ok(url.clone()))
            };
            (Ok(url), merge_result)
        }
        Err(mut failed) => {
            let held = failed.error.is::<crate::failed_run::MergeHold>()
                || failed.error.is::<crate::failed_run::PolicyRefusal>()
                || failed.error.is::<crate::security::review::Hold>();
            if held {
                issue_run.delivered = true;
                let url = failed.pr_url.clone().unwrap_or_default();
                return (Ok(url), Some(Err(failed.error)));
            }
            if let Some(recovery) = &mut failed.recovery {
                recovery.add_command_context(continuation(issue_run), issue_run.on_origin);
                for line in recovery.lines() {
                    progress::step(line);
                }
            }
            (Err(failed.error), None)
        }
    }
}

fn continuation(issue_run: &IssueRun<'_>) -> recovery::Continuation {
    recovery::Continuation {
        command: format!(
            "thirdshift process {} issue {}",
            issue_run.process, issue_run.issue.url
        ),
        launch: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        base: issue_run.base.to_string(),
        branch: issue_run.branch.to_string(),
    }
}

fn preserve_step_failure(
    checkout: &mut Worktree,
    context: recovery::Continuation,
    on_origin: bool,
    error: &anyhow::Error,
) {
    let reason = Cause::of(error).first_line().to_string();
    let preserved = checkout.preserve_failed_run(&context.base, &reason, &mut || Ok(()));
    let preservation = recovery::preservation_of(preserved, progress::step);
    let mut recovery = Recovery::new(recovery::Stage::Implementation, preservation);
    recovery.record_worktree(checkout.path().to_path_buf());
    recovery.add_command_context(context, on_origin);
    for line in recovery.lines() {
        progress::step(line);
    }
}

fn run_step_in(
    run: &StepRun<'_>,
    cwd: &Path,
    step: &ResolvedStep,
    prior: &PriorSteps<'_>,
    results: &Path,
    first: bool,
) -> Result<String> {
    crate::resources::wait()?;
    match &step.kind {
        StepKind::Implement => run_implement(run, cwd, step),
        StepKind::RunPublishedIssue => run_published(run, step, prior),
        StepKind::Skill { name, path } => {
            progress::step(format_args!(
                "running step {} with {choice}",
                step.name,
                choice = run.choice
            ));
            let prompt = prompt::process_step(
                run.process,
                &step.name,
                name,
                path,
                prompt::EarlierSteps {
                    results: prior.results,
                    first,
                },
                run.issue,
                if run.issue_branch {
                    prompt::ProcessWorktree::IssueBranch(
                        run.continuation
                            .as_ref()
                            .map(|(branch, base)| prompt::ProcessContinuation { branch, base }),
                    )
                } else {
                    prompt::ProcessWorktree::Throwaway
                },
            );
            run_skill(run, cwd, step, &prompt)
        }
        StepKind::Delivery => bail!("delivery cannot be in a parallel group"),
        StepKind::SecurityReview => bail!("security-review cannot be in a parallel group"),
        StepKind::SelfMerge => bail!("self-merge must be directly after delivery"),
        StepKind::Command(command) => {
            progress::step(format_args!("running step {}", step.name));
            let result_file = results.join(format!("{}.txt", step.name));
            run_command(
                cwd,
                command,
                results,
                &result_file,
                run.issue.map(|issue| issue.url.as_str()),
            )
        }
    }
}

/// Check the issue, then write it into the worktree for the session about
/// to start. An untrusted edit fails the Step instead of starting it.
fn prepare_for_session(issue: &IssueUrl, started: DateTime<Utc>, worktree: &Path) -> Result<()> {
    delivery::no_untrusted_edit_since(issue, started, "Process")?;
    delivery::hand_over_issue(issue, worktree)
}

/// Record a Step through the shared session lifecycle, including resource
/// admission and the issue trust check before its session and Resume.
fn run_skill(
    run: &StepRun<'_>,
    worktree: &Path,
    step: &ResolvedStep,
    prompt: &str,
) -> Result<String> {
    run_session_step(run, worktree, step, |sessions, index| {
        sessions
            .run_to_final_message(
                session::Purpose::Ordinary,
                session::Kind::ProcessStep(index),
                prompt,
            )?
            .filter(|message| !message.trim().is_empty())
            .with_context(|| format!("Step {} ended without a final message", step.name))
    })
}

fn run_implement(run: &StepRun<'_>, worktree: &Path, step: &ResolvedStep) -> Result<String> {
    let issue = run.issue.context("implement needs an issue")?;
    let checkout = run
        .checkout
        .filter(|_| run.issue_branch)
        .context("implement needs an Issue-branch worktree")?;
    let selection = run.selection.context("implement needs an Issue branch")?;
    let labels = GitHub::new().issue_labels(issue)?;
    let opening = crate::implement::prepare(
        issue,
        run.base,
        checkout,
        delivery::Opening {
            session: delivery::OpeningSession::Implementation,
            prompt: crate::implement::prompt(issue, run.base, selection),
        },
        Some(&labels),
    )?;
    let mut pr = PullRequest::new(issue, checkout.branch(), run.base, GitHub::new());
    run_session_step(run, worktree, step, |sessions, index| {
        let completed = crate::implement::run(issue, &opening, |prompt| {
            let message = sessions.run(session::Kind::ProcessStep(index), prompt)?;
            pr.ensure_actionable(false)?;
            Ok(message)
        })?;
        match completed {
            crate::implement::Completed::Implemented(message) => Ok(message.unwrap_or_default()),
            crate::implement::Completed::NotReproduced(comment_url) => {
                crate::implement::confirm_needs_info(&GitHub::new().issue_labels(issue)?)?;
                let mut completed = run.completed.lock().expect("completed Runs");
                completed.unreproduced_bug = Some(comment_url.clone());
                completed
                    .resolutions
                    .push(crate::run::Resolution::NotReproduced {
                        comment_url: comment_url.clone(),
                    });
                Ok(comment_url)
            }
        }
    })
}

fn run_session_step(
    run: &StepRun<'_>,
    worktree: &Path,
    step: &ResolvedStep,
    action: impl FnOnce(&session::Sessions<'_>, usize) -> Result<String>,
) -> Result<String> {
    let index = run
        .steps
        .iter()
        .position(|other| other.name == step.name)
        .expect("the Step belongs to this Process")
        + 1;
    let logs = session::Logs::of_process(&launch::repo()?);
    let before_each = || match run.issue {
        Some(issue) => prepare_for_session(issue, run.started, worktree),
        None => Ok(()),
    };
    session::Sessions::checked_within(
        &logs,
        worktree,
        run.choice,
        None,
        None,
        &before_each,
        |sessions| {
            let result = action(sessions, index);
            retain_killed_work(run.killed_work, index, sessions);
            result
        },
    )
    .0
}

fn retain_killed_work(
    killed_work: &Mutex<BTreeMap<usize, Vec<String>>>,
    step_index: usize,
    sessions: &session::Sessions<'_>,
) {
    let endings = sessions.take_killed_work();
    if !endings.is_empty() {
        killed_work
            .lock()
            .expect("killed work")
            .insert(step_index, endings);
    }
}

/// Start a Ticket run, or a Spec run, on the issue the session Step before
/// this one published. The child run is part of this Command. Its outcome
/// is this Step's outcome.
fn run_published(run: &StepRun<'_>, step: &ResolvedStep, prior: &PriorSteps<'_>) -> Result<String> {
    let index = run
        .steps
        .iter()
        .position(|other| other.name == step.name)
        .expect("the step is one of the process");
    let Some(previous) = index.checked_sub(1).map(|index| &run.steps[index]) else {
        bail!("run-published-issue must directly follow a session Step");
    };
    let issue = published_issue(previous, prior.results)?;
    let origin = Git::new(std::env::current_dir()?).run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        bail!(
            "the issue {} is not in the repository at origin {origin}",
            issue.url
        );
    }
    let github = GitHub::new();
    let viewed = github.issue(&issue)?;
    if !viewed.is_open {
        bail!("the issue {} is closed", issue.url);
    }
    let association = trust::Reads::author_association(&github, &issue)?;
    if !trust::trusted_association(&association) {
        bail!("#{}'s author is not a Trusted account", issue.number);
    }
    let started = prior
        .began
        .iter()
        .find(|(name, _)| name == &previous.name)
        .map(|(_, at)| *at)
        .with_context(|| format!("step {} has no start time", previous.name))?;
    // GitHub's times are to the second.
    if viewed.created.timestamp() < started.timestamp() {
        bail!(
            "the issue {} was created before the session Step began",
            issue.url
        );
    }
    interrupt::check()?;
    progress::step(format_args!(
        "dispatching {url}, as thirdshift {url} would",
        url = issue.url
    ));
    logs::dispatching(Work::Run(&issue), run.choice, &[]);
    let config = UserConfig::load()?;
    let flags = Flags {
        goal: Some(if run.limits.forbid_self_merge {
            Goal::ReadyForReview
        } else {
            run.goal
        }),
        ..Flags::default()
    };
    let mut asks = Asks::of_run(&issue, &flags, &config);
    asks.delivery_limits = delivery::Limits {
        repair_cap: run.limits.repair_cap,
        protected_paths: run.limits.protected_paths.clone(),
    };
    asks.notification = NotificationAsk::Skip;
    asks.harness = run.choice.clone();
    // `of_run` resolved the review against the default choice. Resolve it
    // again against this Step, so `security.harness` still wins (ADR-0021).
    asks.security_review_harness = Flags::default().security_review_harness(&config, &asks.harness);
    let ended = crate::run::run_to_end(
        &issue,
        &mut asks,
        crate::run::StartedBy::Dispatch { base: run.base },
    );
    logs::dispatch_ended(&ended);
    match ended.outcome {
        Ok(reached) => {
            let url = reached.resolution.url().to_string();
            run.completed
                .lock()
                .expect("completed Runs")
                .resolutions
                .push(reached.resolution);
            Ok(url)
        }
        Err(failed) => {
            run.completed.lock().expect("completed Runs").failed = true;
            Err(failed.error)
        }
    }
}

fn published_issue(previous: &ResolvedStep, earlier: &[(String, PathBuf)]) -> Result<IssueUrl> {
    let Some((_, path)) = earlier.iter().find(|(name, _)| name == &previous.name) else {
        bail!("the session Step ended without a final message");
    };
    let message =
        fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let Some(line) = message
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
    else {
        bail!("the session Step ended without the issue's URL on its last line");
    };
    IssueUrl::parse(line).context("the session Step ended without the issue's URL on its last line")
}

fn resolve_process_issue(issue: Option<IssueInput>) -> Result<Option<IssueUrl>> {
    let Some(issue) = issue else {
        return Ok(None);
    };
    let issue = launch::resolve_issue(issue)?;
    let origin = Git::new(std::env::current_dir()?).run(&["config", "remote.origin.url"])?;
    if !issue.matches_origin(&origin) {
        bail!(
            "origin mismatch: {} is not in the repository at origin {origin}",
            issue.url
        );
    }
    Ok(Some(issue))
}

fn run_command(
    worktree: &Path,
    command: &str,
    results: &Path,
    result_file: &Path,
    issue_url: Option<&str>,
) -> Result<String> {
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(command)
        .current_dir(worktree)
        .env(RESULTS_DIR, results)
        .env(STEP_RESULT, result_file);
    match issue_url {
        Some(url) => {
            child.env(ISSUE_URL, url);
        }
        None => {
            child.env_remove(ISSUE_URL);
        }
    }
    let output = process::output(
        &mut child,
        None,
        Control {
            name: "command",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM]),
        },
    )?;
    if !output.status.success() {
        fs::write(result_file, combined(&output.stdout, &output.stderr))
            .with_context(|| format!("could not write {}", result_file.display()))?;
        bail!("{}", exit_status(output.status));
    }
    Ok(combined(&output.stdout, &output.stderr))
}

fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(stderr));
    text
}

fn exit_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => format!("exit status {status}"),
    }
}

fn worktree_path<'a>(worktrees: &'a [OwnedWorktree], name: Option<&str>) -> &'a Path {
    worktrees
        .iter()
        .find(|worktree| worktree.name.as_deref() == name)
        .unwrap_or_else(|| {
            panic!(
                "missing worktree {}",
                name.unwrap_or("the throwaway worktree")
            )
        })
        .checkout
        .path()
}

fn classify(result: &Result<String>) -> (&'static str, Option<String>) {
    match result {
        Ok(_) => ("succeeded", None),
        Err(error) => (failure_outcome(), Some(one_line(error))),
    }
}

fn failure_outcome() -> &'static str {
    if interrupt::requested() {
        "interrupted"
    } else {
        "failed"
    }
}

fn one_line(error: &anyhow::Error) -> String {
    format!("{error:#}")
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn plan(name: &str, prepared: &Prepared) -> String {
    let mut text = format!(
        "{name}: {description}\n",
        description = prepared.description
    );
    if prepared.declared.is_empty() {
        text.push_str("throwaway worktree at the Base branch\n");
    }
    let _ = writeln!(
        text,
        "harness: {choice} ({chosen})",
        choice = prepared.choice,
        chosen = prepared.choice.chosen_by,
    );
    for worktree in &prepared.declared {
        if worktree.issue_branch {
            let _ = writeln!(text, "worktree {}: Issue branch", worktree.name);
        } else {
            let _ = writeln!(
                text,
                "worktree {}: throwaway at the Base branch",
                worktree.name
            );
        }
    }
    if prepared.limits.repair_cap != delivery::MAX_REPAIRS {
        let _ = writeln!(
            text,
            "limits: repair cap {} of {}",
            prepared.limits.repair_cap,
            delivery::MAX_REPAIRS
        );
    }
    if prepared.limits.forbid_self_merge {
        text.push_str("limits: self-merge held\n");
    }
    if !prepared.limits.protected_paths.is_empty() {
        let _ = writeln!(
            text,
            "limits: protected paths {}",
            prepared.limits.protected_paths.join(", ")
        );
    }
    let named = !prepared.declared.is_empty();
    for range in group_ranges(&prepared.steps) {
        let names: Vec<_> = prepared.steps[range.clone()]
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        for step in &prepared.steps[range.clone()] {
            let _ = writeln!(text, "{}", step_plan(step, named));
            let _ = writeln!(
                text,
                "harness: {} ({})",
                step.choice.harness.name(),
                step.sources.harness
            );
            let _ = writeln!(
                text,
                "model: {} ({})",
                step.choice.model.as_deref().unwrap_or("default"),
                step.sources.model
            );
            let _ = writeln!(
                text,
                "effort: {} ({})",
                step.choice.effort.as_deref().unwrap_or("default"),
                step.sources.effort
            );
            let starts = match &step.start_failure {
                None => "yes".to_string(),
                Some(reason) => format!("no: {reason}"),
            };
            let _ = writeln!(text, "starts: {starts}");
        }
        if names.len() > 1 {
            let _ = writeln!(text, "together: {}", names.join(", "));
        }
    }
    text.trim_end().to_string()
}

fn step_plan(step: &ResolvedStep, named: bool) -> String {
    let mut line = match &step.kind {
        StepKind::Skill { name, path } => {
            format!("{}: {} {}", step.name, name, path.display())
        }
        StepKind::Command(command) => {
            let shown = command.replace(['\n', '\r'], " ");
            format!("{}: command {shown}", step.name)
        }
        StepKind::Delivery => format!("{}: delivery", step.name),
        StepKind::Implement => format!("{}: implement", step.name),
        StepKind::SecurityReview => format!("{}: security-review", step.name),
        StepKind::SelfMerge => format!("{}: self-merge", step.name),
        StepKind::RunPublishedIssue => format!("{}: run-published-issue", step.name),
    };
    if named && let Some(worktree) = &step.worktree {
        let _ = write!(line, " (worktree {worktree})");
    }
    if step.optional {
        line.push_str(" (optional)");
    }
    line
}

fn load(name: &str) -> Result<(PathBuf, ProcessFile)> {
    let (path, text) = Catalog::installed()?.custom(name)?;
    let file = validated(&path, &text)?;
    Ok((path, file))
}

fn validated(path: &Path, text: &str) -> Result<ProcessFile> {
    let file = parse(path, text)?;
    refuse_security_review(path, &file)?;
    refuse_worktrees(path, &file)?;
    refuse_implement(path, &file)?;
    refuse_self_merge(path, &file)?;
    refuse_issue_branch(path, &file)?;
    refuse_published(path, &file.steps)?;
    Ok(file)
}

/// The launch checkout a Process's worktrees are created from, and the
/// Issue-branch checkout a run already holds.
struct WorktreeSource<'a> {
    git: &'a Git,
    commit: &'a str,
    /// Reuse this checkout for the Issue-branch worktree instead of creating it.
    issue_checkout: Option<&'a Path>,
    /// The Base branch checked out in the Launch directory.
    base: &'a str,
}

fn prepare_from(
    path: &Path,
    config: &UserConfig,
    asked: &Asked,
    file: ProcessFile,
    issue: Option<IssueUrl>,
    started: DateTime<Utc>,
    source: WorktreeSource<'_>,
) -> Result<Prepared> {
    let issue_worktree = file
        .worktrees
        .iter()
        .find_map(|worktree| worktree.issue_branch_line.map(|_| worktree.name.clone()));
    let declared = file
        .worktrees
        .iter()
        .map(|worktree| DeclaredWorktree {
            name: worktree.name.clone(),
            issue_branch: worktree.issue_branch_line.is_some(),
        })
        .collect();
    let skip = source.issue_checkout.and(issue_worktree.as_deref());
    let worktrees = create_worktrees(source.git, source.commit, &file, skip)?;
    let mut command = CommandChoice::of(asked, &config.harness);
    let mut steps = Vec::new();
    for step in &file.steps {
        let checkout = resolve_dir(
            &worktrees,
            step,
            issue_worktree.as_deref(),
            source.issue_checkout,
        );
        steps.push(resolve(path, step, checkout, &command)?);
    }
    command.check_steps(&mut steps);
    for step in &mut steps {
        if matches!(step.kind, StepKind::SecurityReview) {
            step.check_security_review(file.security_review, config);
        }
    }
    Ok(Prepared {
        path: path.to_path_buf(),
        worktrees,
        declared,
        issue_worktree,
        _written: WrittenGuard,
        choice: command.choice,
        description: file.description,
        steps,
        issue,
        started,
        base: source.base.to_string(),
        limits: file.limits,
        goal: file.goal,
        security_review: file.security_review,
        completed: Mutex::new(CompletedRuns::default()),
        killed_work: Mutex::default(),
    })
}

fn resolve_dir<'a>(
    worktrees: &'a [OwnedWorktree],
    step: &StepDraft,
    issue_worktree: Option<&str>,
    issue_checkout: Option<&'a Path>,
) -> &'a Path {
    if let Some(checkout) = issue_checkout
        && names_issue_worktree(
            step.worktree.as_ref().map(|(name, _)| name.as_str()),
            issue_worktree,
        )
    {
        return checkout;
    }
    worktree_path(
        worktrees,
        step.worktree.as_ref().map(|(name, _)| name.as_str()),
    )
}

fn create_worktrees(
    launch: &Git,
    commit: &str,
    file: &ProcessFile,
    skip: Option<&str>,
) -> Result<Vec<OwnedWorktree>> {
    if file.worktrees.is_empty() {
        return Ok(vec![OwnedWorktree {
            name: None,
            checkout: SkillWorktree::at(launch, commit)?,
        }]);
    }
    let mut worktrees = Vec::new();
    let mut seen = BTreeSet::new();
    for step in &file.steps {
        let Some((name, _)) = &step.worktree else {
            continue;
        };
        if Some(name.as_str()) == skip || !seen.insert(name.clone()) {
            continue;
        }
        worktrees.push(OwnedWorktree {
            name: Some(name.clone()),
            checkout: SkillWorktree::at(launch, commit)?,
        });
    }
    Ok(worktrees)
}

fn refuse_worktrees(path: &Path, file: &ProcessFile) -> Result<()> {
    let declared: BTreeSet<&str> = file
        .worktrees
        .iter()
        .map(|worktree| worktree.name.as_str())
        .collect();
    for step in &file.steps {
        match &step.worktree {
            Some((name, line)) if !declared.contains(name.as_str()) => {
                return Err(rule(
                    path,
                    *line,
                    "unknown worktree",
                    format!("step {} names worktree {name}", step.name),
                ));
            }
            None if !declared.is_empty() => {
                return Err(rule(
                    path,
                    step.name_line,
                    "worktree",
                    format!("step {} does not name a worktree", step.name),
                ));
            }
            _ => {}
        }
    }
    for range in consecutive_groups(&file.steps, |step| {
        step.parallel.as_ref().map(|(name, _)| name.as_str())
    }) {
        if range.len() < 2 {
            continue;
        }
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for step in &file.steps[range] {
            let worktree = step
                .worktree
                .as_ref()
                .map(|(name, _)| name.as_str())
                .unwrap_or("");
            if let Some(other) = seen
                .iter()
                .find(|(name, _)| *name == worktree)
                .map(|(_, step)| *step)
            {
                let line = step
                    .worktree
                    .as_ref()
                    .map(|(_, line)| *line)
                    .or_else(|| step.parallel.as_ref().map(|(_, line)| *line))
                    .unwrap_or(step.name_line);
                let shared = if worktree.is_empty() {
                    "the throwaway worktree".to_string()
                } else {
                    format!("worktree {worktree}")
                };
                return Err(rule(
                    path,
                    line,
                    "shared worktree",
                    format!("steps {other} and {} share {shared}", step.name),
                ));
            }
            seen.push((worktree, step.name.as_str()));
        }
    }
    Ok(())
}

fn file_has_issue_branch(file: &ProcessFile) -> bool {
    file.worktrees
        .iter()
        .any(|worktree| worktree.issue_branch_line.is_some())
}

fn refuse_implement(path: &Path, file: &ProcessFile) -> Result<()> {
    let issue = file
        .worktrees
        .iter()
        .find(|worktree| worktree.issue_branch_line.is_some());
    for step in &file.steps {
        if let DraftKind::Implement { line } = step.kind
            && !issue.is_some_and(|issue| {
                step.worktree
                    .as_ref()
                    .is_some_and(|(name, _)| name == &issue.name)
            })
        {
            return Err(rule(
                path,
                line,
                "implement",
                "implement needs an Issue-branch worktree",
            ));
        }
    }
    Ok(())
}

fn require_issue(path: &Path, file: &ProcessFile, issue: Option<&IssueUrl>) -> Result<()> {
    if issue.is_some() {
        return Ok(());
    }
    let Some(worktree) = file
        .worktrees
        .iter()
        .find(|worktree| worktree.issue_branch_line.is_some())
    else {
        return Ok(());
    };
    Err(rule(
        path,
        worktree.issue_branch_line.expect("issue-branch line"),
        "issue",
        "an Issue-branch worktree needs an issue",
    ))
}

fn refuse_self_merge(path: &Path, file: &ProcessFile) -> Result<()> {
    let issue_branch = file
        .worktrees
        .iter()
        .find(|worktree| worktree.issue_branch_line.is_some());
    let mut seen = false;
    for (index, step) in file.steps.iter().enumerate() {
        let DraftKind::SelfMerge { line } = step.kind else {
            continue;
        };
        if seen {
            return Err(rule(
                path,
                line,
                "self-merge",
                "a Process has at most one self-merge Step",
            ));
        }
        seen = true;
        let follows = index > 0
            && matches!(file.steps[index - 1].kind, DraftKind::Delivery { .. })
            && !same_group(&file.steps[index - 1], step);
        if !follows {
            return Err(rule(
                path,
                line,
                "self-merge",
                "self-merge must be directly after delivery",
            ));
        }
        if step.optional {
            return Err(rule(
                path,
                line,
                "self-merge",
                "self-merge cannot be optional",
            ));
        }
        if let Some((_, parallel_line)) = &step.parallel {
            return Err(rule(
                path,
                *parallel_line,
                "self-merge",
                "self-merge cannot be in a parallel group",
            ));
        }
        let uses_issue = step
            .worktree
            .as_ref()
            .is_some_and(|(name, _)| issue_branch.is_some_and(|branch| branch.name == *name));
        if !uses_issue {
            let reported = step
                .worktree
                .as_ref()
                .map(|(_, worktree_line)| *worktree_line)
                .unwrap_or(line);
            return Err(rule(
                path,
                reported,
                "self-merge",
                "self-merge must use the Issue-branch worktree",
            ));
        }
    }
    Ok(())
}

fn refuse_security_review(path: &Path, file: &ProcessFile) -> Result<()> {
    let issue_branch = file
        .worktrees
        .iter()
        .find(|worktree| worktree.issue_branch_line.is_some());
    let delivery_at = file
        .steps
        .iter()
        .position(|step| matches!(step.kind, DraftKind::Delivery { .. }));
    for (index, step) in file.steps.iter().enumerate() {
        let DraftKind::SecurityReview { line } = step.kind else {
            continue;
        };
        let Some(issue_branch) = issue_branch else {
            return Err(rule(
                path,
                line,
                "security-review",
                "security-review needs an Issue-branch worktree",
            ));
        };
        if let Some((_, parallel_line)) = &step.parallel {
            return Err(rule(
                path,
                *parallel_line,
                "security-review",
                "security-review cannot be in a parallel group",
            ));
        }
        if delivery_at.is_some_and(|at| index > at) {
            return Err(rule(
                path,
                line,
                "security-review",
                "security-review must precede delivery",
            ));
        }
        if !step
            .worktree
            .as_ref()
            .is_some_and(|(name, _)| name == &issue_branch.name)
        {
            return Err(rule(
                path,
                line,
                "security-review",
                "security-review must use the Issue-branch worktree",
            ));
        }
        if file.steps[index + 1..].iter().any(|later| {
            !matches!(
                later.kind,
                DraftKind::SecurityReview { .. }
                    | DraftKind::Delivery { .. }
                    | DraftKind::SelfMerge { .. }
            ) && later
                .worktree
                .as_ref()
                .is_some_and(|(name, _)| name == &issue_branch.name)
        }) {
            return Err(rule(
                path,
                line,
                "security-review",
                "security-review must follow the editing Steps",
            ));
        }
    }
    Ok(())
}

fn refuse_issue_branch(path: &Path, file: &ProcessFile) -> Result<()> {
    let mut issue_branches = file
        .worktrees
        .iter()
        .filter(|worktree| worktree.issue_branch_line.is_some());
    let Some(issue_branch) = issue_branches.next() else {
        for step in &file.steps {
            if let DraftKind::Delivery { line } = &step.kind {
                return Err(rule(
                    path,
                    *line,
                    "delivery",
                    "delivery needs an Issue-branch worktree",
                ));
            }
        }
        return Ok(());
    };
    if let Some(second) = issue_branches.next() {
        return Err(rule(
            path,
            second.issue_branch_line.expect("issue-branch line"),
            "issue-branch",
            "a Process has at most one Issue-branch worktree",
        ));
    }
    let mut delivery_at = None;
    let mut last_use = None;
    for (index, step) in file.steps.iter().enumerate() {
        if let DraftKind::Delivery { line } = &step.kind {
            if delivery_at.is_some() {
                return Err(rule(
                    path,
                    *line,
                    "delivery",
                    "a Process has at most one delivery Step",
                ));
            }
            if step.optional {
                return Err(rule(path, *line, "delivery", "delivery cannot be optional"));
            }
            if let Some((_, parallel_line)) = &step.parallel {
                return Err(rule(
                    path,
                    *parallel_line,
                    "delivery",
                    "delivery cannot be in a parallel group",
                ));
            }
            let uses = step
                .worktree
                .as_ref()
                .is_some_and(|(name, _)| name == &issue_branch.name);
            if !uses {
                let line = step
                    .worktree
                    .as_ref()
                    .map(|(_, line)| *line)
                    .unwrap_or(*line);
                return Err(rule(
                    path,
                    line,
                    "delivery",
                    "delivery must use the Issue-branch worktree",
                ));
            }
            delivery_at = Some(index);
        } else if matches!(step.kind, DraftKind::SelfMerge { .. }) {
            // Self-merge publishes; it is not an editing use of the worktree.
        } else if step
            .worktree
            .as_ref()
            .is_some_and(|(name, _)| name == &issue_branch.name)
        {
            last_use = Some(index);
        }
    }
    match last_use {
        Some(use_at) if delivery_at.is_some_and(|at| at > use_at) => Ok(()),
        Some(use_at) => {
            let step = &file.steps[use_at];
            let line = step
                .worktree
                .as_ref()
                .map(|(_, line)| *line)
                .unwrap_or(step.name_line);
            Err(rule(
                path,
                line,
                "delivery",
                format!(
                    "no delivery Step after step {}, which uses the Issue-branch worktree",
                    step.name
                ),
            ))
        }
        None if delivery_at.is_some() => Ok(()),
        None => Err(rule(
            path,
            issue_branch.issue_branch_line.expect("issue-branch line"),
            "delivery",
            "an Issue-branch worktree needs a delivery Step after the last Step that uses it",
        )),
    }
}

fn refuse_published(path: &Path, steps: &[StepDraft]) -> Result<()> {
    for (index, step) in steps.iter().enumerate() {
        let DraftKind::RunPublishedIssue { line } = step.kind else {
            continue;
        };
        let follows = index.checked_sub(1).is_some_and(|previous| {
            matches!(steps[previous].kind, DraftKind::Skill { .. })
                && !same_group(&steps[previous], step)
        });
        if !follows {
            return Err(rule(
                path,
                line,
                "run-published-issue",
                "run-published-issue must directly follow a session Step",
            ));
        }
    }
    Ok(())
}

fn same_group(earlier: &StepDraft, step: &StepDraft) -> bool {
    match (&earlier.parallel, &step.parallel) {
        (Some((earlier, _)), Some((step, _))) => earlier == step,
        _ => false,
    }
}

fn resolve(
    path: &Path,
    step: &StepDraft,
    worktree: &Path,
    command: &CommandChoice,
) -> Result<ResolvedStep> {
    let kind = match &step.kind {
        DraftKind::Skill { skill, line } => match skills::find(worktree, skill) {
            Ok(skill) => StepKind::Skill {
                name: skill.name,
                path: skill.path,
            },
            Err(error) => {
                return Err(rule(path, *line, "skill not found", format!("{error:#}")));
            }
        },
        DraftKind::Command { command } => StepKind::Command(command.clone()),
        DraftKind::Implement { .. } => StepKind::Implement,
        DraftKind::SecurityReview { .. } => StepKind::SecurityReview,
        DraftKind::Delivery { .. } => StepKind::Delivery,
        DraftKind::SelfMerge { .. } => StepKind::SelfMerge,
        DraftKind::RunPublishedIssue { .. } => StepKind::RunPublishedIssue,
    };
    let (choice, sources) = command.step(step);
    Ok(ResolvedStep {
        name: step.name.clone(),
        optional: step.optional,
        worktree: step.worktree.as_ref().map(|(name, _)| name.clone()),
        parallel: step.parallel.as_ref().map(|(name, _)| name.clone()),
        kind,
        report_line: step.report_line,
        choice,
        sources,
        start_failure: None,
        review_default: None,
    })
}

fn parse(path: &Path, text: &str) -> Result<ProcessFile> {
    let doc = Document::parse(text).map_err(|error: toml_edit::TomlError| {
        let line = error
            .span()
            .map(|span| line_of(text, span.start))
            .unwrap_or(1);
        rule(path, line, "syntax", error)
    })?;
    let table = doc.as_table();
    let mut saw_version = false;
    let mut description = None;
    let mut worktrees = Vec::new();
    let mut steps = None;
    let mut limits = Limits::open();
    for (key, item) in table.iter() {
        let line = key_line(text, table.key(key));
        match key {
            "version" => {
                let Some(value) = item.as_integer() else {
                    return Err(rule(path, line, "version", "version must be the integer 1"));
                };
                if value != 1 {
                    return Err(rule(path, line, "version", "version must be 1"));
                }
                saw_version = true;
            }
            "description" => {
                description = Some(description_of(path, line, item)?);
            }
            "worktrees" => {
                worktrees = parse_worktrees(path, text, line, item)?;
            }
            "steps" => {
                let parsed = parse_steps(path, text, line, item)?;
                if parsed.is_empty() {
                    return Err(rule(path, line, "steps", "at least one step is required"));
                }
                steps = Some(parsed);
            }
            "limits" => limits = parse_limits(path, text, line, item)?,
            other => return Err(rule(path, line, "unknown key", other)),
        }
    }
    if !saw_version {
        return Err(rule(path, 1, "version", "version = 1 is required"));
    }
    let Some(description) = description else {
        return Err(rule(path, 1, "description", "description is required"));
    };
    let Some(steps) = steps else {
        return Err(rule(path, 1, "steps", "at least one step is required"));
    };
    Ok(ProcessFile {
        description,
        worktrees,
        steps,
        limits,
        goal: Goal::ReadyForReview,
        security_review: false,
    })
}

fn parse_limits(path: &Path, text: &str, line: usize, item: &Item) -> Result<Limits> {
    let Some(table) = item.as_table() else {
        return Err(rule(path, line, "limits", "limits must be a table"));
    };
    let mut limits = Limits::open();
    for (key, value) in table.iter() {
        let field_line = key_line(text, table.key(key));
        match key {
            "repair_cap" => limits.repair_cap = repair_cap_of(path, field_line, value)?,
            "self_merge" => limits.forbid_self_merge = forbid_self_merge(path, field_line, value)?,
            "protected_paths" => {
                limits.protected_paths = protected_paths_of(path, text, field_line, value)?;
            }
            other => return Err(rule(path, field_line, "unknown key", other)),
        }
    }
    Ok(limits)
}

fn repair_cap_of(path: &Path, line: usize, item: &Item) -> Result<usize> {
    let whole = format!(
        "repair_cap must be a whole number from 0 to {}",
        delivery::MAX_REPAIRS
    );
    let Some(value) = item.as_integer() else {
        return Err(rule(path, line, "repair_cap", whole));
    };
    if value < 0 {
        return Err(rule(path, line, "repair_cap", whole));
    }
    if value > delivery::MAX_REPAIRS as i64 {
        return Err(rule(
            path,
            line,
            "loosening limit",
            format!(
                "repair_cap {value} is above the effective cap {}",
                delivery::MAX_REPAIRS
            ),
        ));
    }
    Ok(value as usize)
}

/// `self_merge = false` forbids a merge. `true` would loosen the built-in rule
/// that a merge happens only when the command or the User config asks.
fn forbid_self_merge(path: &Path, line: usize, item: &Item) -> Result<bool> {
    let Some(value) = item.as_bool() else {
        return Err(rule(path, line, "self_merge", "self_merge must be false"));
    };
    if value {
        return Err(rule(
            path,
            line,
            "loosening limit",
            "self_merge = true would allow a merge",
        ));
    }
    Ok(true)
}

fn protected_paths_of(path: &Path, text: &str, line: usize, item: &Item) -> Result<Vec<String>> {
    let Some(array) = item.as_array() else {
        return Err(rule(
            path,
            line,
            "protected_paths",
            "protected_paths must be an array of globs",
        ));
    };
    let mut paths = Vec::new();
    for value in array.iter() {
        let value_line = value
            .span()
            .map(|span| line_of(text, span.start))
            .unwrap_or(line);
        let Some(glob) = value.as_str() else {
            return Err(rule(
                path,
                value_line,
                "protected_paths",
                "protected_paths must be an array of globs",
            ));
        };
        if glob.is_empty() {
            return Err(rule(
                path,
                value_line,
                "protected path",
                "a protected path glob must not be empty",
            ));
        }
        refuse_controls(path, value_line, "protected path", glob, false)?;
        paths.push(glob.to_string());
    }
    Ok(paths)
}

fn parse_worktrees(
    path: &Path,
    text: &str,
    line: usize,
    item: &Item,
) -> Result<Vec<WorktreeDraft>> {
    let Some(table) = item.as_table() else {
        return Err(rule(
            path,
            line,
            "worktrees",
            "worktrees must be a table of named worktrees",
        ));
    };
    if table.is_empty() {
        return Err(rule(
            path,
            line,
            "worktrees",
            "at least one worktree is required",
        ));
    }
    let mut worktrees = Vec::new();
    for (key, item) in table.iter() {
        let name_line = key_line(text, table.key(key));
        if !crate::args::one_path_component(key) {
            return Err(rule(
                path,
                name_line,
                "worktree name",
                "a worktree name must be a single path component",
            ));
        }
        refuse_controls(path, name_line, "worktree name", key, false)?;
        let Some(body) = item.as_table() else {
            return Err(rule(
                path,
                name_line,
                "worktree",
                "a worktree must be a table",
            ));
        };
        let mut saw_kind = false;
        let mut issue_branch_line = None;
        for (field, value) in body.iter() {
            let field_line = key_line(text, body.key(field));
            match field {
                "kind" => {
                    let Some(kind) = value.as_str() else {
                        return Err(rule(
                            path,
                            field_line,
                            "worktree kind",
                            "kind must be a string",
                        ));
                    };
                    if kind != "throwaway" && kind != "issue-branch" {
                        return Err(rule(
                            path,
                            field_line,
                            "worktree kind",
                            "kind must be throwaway or issue-branch",
                        ));
                    }
                    saw_kind = true;
                    issue_branch_line = (kind == "issue-branch").then_some(field_line);
                }
                other => return Err(rule(path, field_line, "unknown key", other)),
            }
        }
        if !saw_kind {
            return Err(rule(path, name_line, "worktree kind", "kind is required"));
        }
        worktrees.push(WorktreeDraft {
            name: key.to_string(),
            issue_branch_line,
        });
    }
    Ok(worktrees)
}

fn description_of(path: &Path, line: usize, item: &Item) -> Result<String> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "description",
            "description must be a string",
        ));
    };
    if value.is_empty() {
        return Err(rule(
            path,
            line,
            "description",
            "description must not be empty",
        ));
    }
    // A newline keeps the one-line rule. Any other control character,
    // including a carriage return or a tab, is refused on its own.
    if value.contains('\n') {
        return Err(rule(
            path,
            line,
            "description",
            "description must be one line",
        ));
    }
    refuse_controls(path, line, "description", value, false)?;
    Ok(value.to_string())
}

fn parse_steps(path: &Path, text: &str, line: usize, item: &Item) -> Result<Vec<StepDraft>> {
    let Some(tables) = item.as_array_of_tables() else {
        return Err(rule(
            path,
            line,
            "steps",
            "steps must be an array of tables",
        ));
    };
    let mut steps = Vec::new();
    let mut names = BTreeSet::new();
    for table in tables.iter() {
        let table_line = table
            .span()
            .map(|span| line_of(text, span.start))
            .unwrap_or(line);
        let step = parse_step(path, text, table_line, table)?;
        if !names.insert(step.name.clone()) {
            return Err(rule(
                path,
                step.name_line,
                "step name",
                format!("duplicate step {}", step.name),
            ));
        }
        steps.push(step);
    }
    Ok(steps)
}

fn parse_step(path: &Path, text: &str, table_line: usize, table: &Table) -> Result<StepDraft> {
    let mut name = None;
    let mut kinds = Vec::new();
    let mut worktree = None;
    let mut parallel = None;
    let mut optional = false;
    let mut harness = None;
    let mut model = None;
    let mut effort = None;
    for (key, item) in table.iter() {
        let line = key_line(text, table.key(key));
        match key {
            "name" => name = Some(step_name(path, line, item)?),
            "skill" => {
                let (skill, line) = step_skill(path, line, item)?;
                kinds.push((DraftKind::Skill { skill, line }, line));
            }
            "command" => {
                let (command, line) = step_command(path, line, item)?;
                kinds.push((DraftKind::Command { command }, line));
            }
            "catalogue" => kinds.push(step_catalogue(path, line, item)?),
            "worktree" => worktree = Some(step_worktree(path, line, item)?),
            "parallel" => parallel = Some(step_parallel(path, line, item)?),
            "optional" => optional = step_optional(path, line, item)?,
            "harness" => harness = Some(step_harness(path, line, item)?),
            "model" => model = Some(step_word(path, line, item, "model")?),
            "effort" => effort = Some(step_word(path, line, item, "effort")?),
            other => return Err(rule(path, line, "unknown key", other)),
        }
    }
    let Some((name, name_line)) = name else {
        return Err(rule(path, table_line, "step name", "a step needs a name"));
    };
    if kinds.len() > 1 {
        return Err(rule(
            path,
            kinds[1].1,
            "exactly one kind",
            format!("step {name} has more than one kind"),
        ));
    }
    let Some((kind, _)) = kinds.pop() else {
        return Err(rule(
            path,
            name_line,
            "exactly one kind",
            format!("step {name} has no kind"),
        ));
    };
    let (harness, report_line) = match harness {
        Some((harness, line)) => (Some(harness), line),
        None => (None, name_line),
    };
    Ok(StepDraft {
        name,
        name_line,
        optional,
        worktree,
        parallel,
        kind,
        harness,
        report_line,
        model,
        effort,
    })
}

fn step_harness(path: &Path, line: usize, item: &Item) -> Result<(Harness, usize)> {
    let value = step_word(path, line, item, "harness")?;
    let Some(harness) = Harness::named(&value) else {
        return Err(rule(path, line, "unknown harness", &value));
    };
    Ok((harness, line))
}

fn step_word(path: &Path, line: usize, item: &Item, rule_name: &str) -> Result<String> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            rule_name,
            format!("{rule_name} must be a string"),
        ));
    };
    if value.trim().is_empty() {
        return Err(rule(
            path,
            line,
            rule_name,
            format!("{rule_name} must not be empty"),
        ));
    }
    refuse_controls(path, line, rule_name, value, false)?;
    Ok(value.to_string())
}

fn step_worktree(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "worktree", "worktree must be a string"));
    };
    if !crate::args::one_path_component(value) {
        return Err(rule(
            path,
            line,
            "worktree",
            "worktree must be a single path component",
        ));
    }
    refuse_controls(path, line, "worktree", value, false)?;
    Ok((value.to_string(), line))
}

fn step_parallel(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "parallel", "parallel must be a string"));
    };
    if value.is_empty() || value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(rule(
            path,
            line,
            "parallel",
            "parallel must be a single-line group name",
        ));
    }
    refuse_controls(path, line, "parallel", value, false)?;
    Ok((value.to_string(), line))
}

fn step_optional(path: &Path, line: usize, item: &Item) -> Result<bool> {
    item.as_bool()
        .ok_or_else(|| rule(path, line, "optional", "optional must be a boolean"))
}

fn step_name(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "step name", "name must be a string"));
    };
    if !crate::args::one_path_component(value) {
        return Err(rule(
            path,
            line,
            "step name",
            "name must be a single path component",
        ));
    }
    refuse_controls(path, line, "name", value, false)?;
    Ok((value.to_string(), line))
}

fn step_skill(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "skill must be a string",
        ));
    };
    refuse_controls(path, line, "skill", value, false)?;
    Ok((value.to_string(), line))
}

fn step_catalogue(path: &Path, line: usize, item: &Item) -> Result<(DraftKind, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "catalogue must be a string",
        ));
    };
    let kind = match value {
        "implement" => DraftKind::Implement { line },
        "delivery" => DraftKind::Delivery { line },
        "security-review" => DraftKind::SecurityReview { line },
        "run-published-issue" => DraftKind::RunPublishedIssue { line },
        "self-merge" => DraftKind::SelfMerge { line },
        _ => {
            return Err(rule(
                path,
                line,
                "catalogue",
                "catalogue must be implement, delivery, run-published-issue, security-review or self-merge",
            ));
        }
    };
    Ok((kind, line))
}

fn step_command(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "command must be a string",
        ));
    };
    if value.is_empty() {
        return Err(rule(
            path,
            line,
            "exactly one kind",
            "command must not be empty",
        ));
    }
    // Newlines and tabs are script text. Any other control character is
    // refused so `check` cannot print a sequence that erases the plan.
    refuse_controls(path, line, "command", value, true)?;
    Ok((value.to_string(), line))
}

/// `check` prints these strings. A control character in one can erase that
/// plan in a terminal while the command still runs. A command may contain
/// newlines and tabs; nothing else may.
fn refuse_controls(
    path: &Path,
    line: usize,
    field: &str,
    value: &str,
    allows_line_break: bool,
) -> Result<()> {
    let forbidden = value
        .chars()
        .any(|c| c.is_control() && !(allows_line_break && (c == '\n' || c == '\t')));
    if forbidden {
        return Err(rule(
            path,
            line,
            "control character",
            format!("{field} must not contain a control character"),
        ));
    }
    Ok(())
}

fn line_of(text: &str, offset: usize) -> usize {
    text.bytes()
        .take(offset)
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn key_line(text: &str, key: Option<&Key>) -> usize {
    key.and_then(Key::span)
        .map(|span| line_of(text, span.start))
        .unwrap_or(1)
}

fn rule(path: &Path, line: usize, rule: &str, detail: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{path}:{line}: {rule}: {detail}", path = path.display())
}
