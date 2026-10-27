//! Custom processes: skill, command and catalogue Steps from
//! `~/.thirdshift/processes/<name>.toml`.
//!
//! Steps run in order. Consecutive Steps that name the same `parallel` group
//! run at the same time, each in a throwaway worktree the Process declares.
//! One worktree may be an Issue branch. The Steps that use it are the editing
//! Steps, and a `delivery` Step after the last of them publishes that branch
//! and leaves a pull request ready for review.
//!
//! A Process started with no issue is a Pass, unless it declares an
//! Issue-branch worktree, which needs an issue. A Pass takes the repository's
//! Pass lock and is skipped, before any worktree, while another Pass holds
//! it. It does not take an Architect run's Ready-issue or Architect-idea
//! gates. A Process started with an issue is not a Pass and does not take the
//! lock.

use std::collections::BTreeSet;
use std::fmt::Write;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use toml_edit::{Document, Item, Key, Table};

use crate::args::ProcessAction;
use crate::asks::Flags;
use crate::base_fix::{BaseFix, BaseFixAsk};
use crate::branch::{self, Selection};
use crate::child_run::ChildAsks;
use crate::claim::{self, Claim};
use crate::config::UserConfig;
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::{Asked, Choice};
use crate::issue::{IssueInput, IssueUrl};
use crate::logs::{self, Begin, Work};
use crate::notification::{About, NotificationAsk, RunNotification};
use crate::process::{self, Control, Interruption};
use crate::pull_request::PullRequest;
use crate::recovery::{self, Recovery};
use crate::run::Goal;
use crate::run_ending::{Account, Cause};
use crate::skill;
use crate::worktree::{SkillWorktree, Worktree};
use crate::{delivery, interrupt, launch, progress, prompt, session, skills, trust};

/// The issue URL when the Process has an issue; unset otherwise.
const ISSUE_URL: &str = "THIRDSHIFT_ISSUE_URL";
/// The folder of Step result files, beside the Command log.
const RESULTS_DIR: &str = "THIRDSHIFT_RESULTS_DIR";
/// The path thirdshift writes this Step's combined output to when it finishes.
const STEP_RESULT: &str = "THIRDSHIFT_STEP_RESULT";

const RESERVED: &[&str] = &[
    "check",
    "list",
    "ticket-run",
    "spec-run",
    "architect-run",
    "pickup-run",
    "security-run",
];

struct ProcessFile {
    description: String,
    worktrees: Vec<WorktreeDraft>,
    steps: Vec<StepDraft>,
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
}

enum DraftKind {
    Skill { skill: String, line: usize },
    Command { command: String },
    Catalogue { line: usize },
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
}

enum StepKind {
    Skill { name: String, path: PathBuf },
    Command(String),
    Delivery,
}

struct OwnedWorktree {
    name: Option<String>,
    checkout: SkillWorktree,
}

struct Prepared {
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
}

/// The Issue branch a run is delivering, beside the throwaway worktrees.
struct IssueRun<'a> {
    checkout: &'a mut Option<Worktree>,
    issue: &'a IssueUrl,
    branch: &'a str,
    base: &'a str,
    on_origin: bool,
    logs: &'a session::Logs,
    base_fix: &'a mut BaseFix,
    process: &'a str,
}

/// Ends the Claim on drop unless the run removed it or ended it already.
struct HeldClaim<'a> {
    claim: Option<Claim<'a>>,
}

impl HeldClaim<'_> {
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
    choice: &'a Choice,
    issue: Option<&'a IssueUrl>,
    started: DateTime<Utc>,
    /// Branch and Base branch when this Step continues an Issue branch.
    continuation: Option<(String, String)>,
}

struct WrittenGuard;

impl Drop for WrittenGuard {
    fn drop(&mut self) {
        skills::remove_written();
    }
}

pub fn execute(action: ProcessAction) -> Result<String> {
    match action {
        ProcessAction::List => list(),
        ProcessAction::Check { name, issue } => check(&name, issue),
        ProcessAction::Run { name, issue, email } => run(&name, issue, email),
    }
}

fn list() -> Result<String> {
    let dir = processes_dir()?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not list {}", dir.display()));
        }
    };
    let mut lines = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("could not list {}", dir.display()))?;
        let file_name = entry.file_name();
        let Some(name) = file_name
            .to_str()
            .and_then(|name| name.strip_suffix(".toml"))
        else {
            continue;
        };
        if !single_component(name) || reserved(name) {
            continue;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(file) = parse(&path, &text) else {
            continue;
        };
        lines.push(format!("{name}: {}", file.description));
    }
    lines.sort();
    Ok(lines.join("\n"))
}

fn check(name: &str, issue: Option<IssueInput>) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let (path, file) = load(name)?;
    let issue = resolve_process_issue(issue)?;
    require_issue(&path, &file, issue.as_ref())?;
    let base = launch::checked_out_base()?;
    let launch = Git::new(std::env::current_dir()?);
    let prepared = prepare_from(
        &path,
        &config,
        file,
        issue,
        Utc::now(),
        &launch,
        base.origin_commit(),
        None,
    )?;
    Ok(plan(name, &prepared))
}

fn run(name: &str, issue: Option<IssueInput>, email: Option<NotificationAsk>) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    // The same check as a Ticket run, before any worktree or Step: a missing
    // address or Resend key fails the Process before it starts. As for the
    // other Passes, the check is before a skip, so a Process that would be
    // skipped fails on it too.
    let notification = RunNotification::new(
        Flags {
            email,
            ..Flags::default()
        }
        .notification(&config),
        &config.email,
        About::Process { name },
    )?;
    let (path, file) = match load(name) {
        Ok(loaded) => loaded,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
    // Before any worktree: a foreign Issue URL is an origin mismatch, and a
    // number that cannot be resolved stops the same way. An Issue-branch
    // worktree with no issue is refused before the Pass log. A failure here
    // is still a failed Process, so it sends the Run notification a Ticket
    // run would send for the same kind of stop.
    let issue = match resolve_process_issue(issue) {
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
                let choice = Choice::of(&Asked::default(), &config.harness);
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
            file,
            None,
            started,
            &launch,
            base.origin_commit(),
            None,
        ) {
            Ok(prepared) => prepared,
            Err(error) => return Err(failed_before_steps(notification, error)),
        };
        return finish_run(name, &repo, prepared, notification);
    }
    let issue = issue.expect("an issue");
    let started = Utc::now() - crate::run::STARTED_MARGIN;
    if file_has_issue_branch(&file) {
        return run_issue_branch(name, &config, &path, file, issue, started, notification);
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
        file,
        Some(issue),
        started,
        &launch,
        base.origin_commit(),
        None,
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
    notify(notification, "failed", Some(&cause));
    error
}

/// Save and, when asked, send the Run notification. A skipped Process sends none.
fn notify(notification: RunNotification, outcome: &'static str, cause: Option<&str>) {
    if outcome == "skipped" {
        return;
    }
    let ended = if outcome == "succeeded" {
        Ok(outcome.to_string())
    } else {
        Err(Cause::of(&anyhow!("{}", cause.unwrap_or(outcome))))
    };
    let account = Account {
        recovery: None,
        outcome,
        ended,
        interrupted: outcome == "interrupted",
        pr_url: None,
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
        urls: Vec::new(),
    };
    notification.finish(&account);
}

fn finish_run(
    name: &str,
    repo: &crate::issue::Repo,
    prepared: Prepared,
    mut notification: RunNotification,
) -> Result<String> {
    logs::started(Work::Process { repo, name }, &prepared.choice);
    notification.built_with(&prepared.choice);
    let result = execute_steps(name, &prepared, None);
    let (outcome, cause) = classify(&result);
    drop(prepared);
    logs::ended(&outcome);
    logs::complete(&outcome, cause.as_deref());
    notify(notification, outcome, cause.as_deref());
    result
}

fn run_issue_branch(
    name: &str,
    config: &UserConfig,
    path: &Path,
    file: ProcessFile,
    issue: IssueUrl,
    started: DateTime<Utc>,
    mut notification: RunNotification,
) -> Result<String> {
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
        file,
        Some(issue.clone()),
        started,
        &launch_git,
        &commit,
        Some(checkout.path()),
    ) {
        Ok(prepared) => prepared,
        Err(error) => return Err(failed_before_steps(notification, error)),
    };
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
    let result = {
        let mut issue_run = IssueRun {
            checkout: &mut held,
            issue: &issue,
            branch: &branch,
            base: &base,
            on_origin,
            logs: &logs,
            base_fix: &mut base_fix,
            process: name,
        };
        let result = execute_steps(name, &prepared, Some(&mut issue_run));
        let context = continuation(&issue_run);
        let kept_on_origin = issue_run.on_origin;
        if let Err(error) = &result
            && let Some(checkout) = issue_run.checkout.as_mut()
        {
            preserve_step_failure(checkout, context, kept_on_origin, error);
        }
        result
    };
    if result.is_ok() {
        claim.remove();
    } else {
        claim.release();
    }
    let (outcome, cause) = classify(&result);
    drop(prepared);
    drop(held);
    logs::ended(&outcome);
    logs::complete(&outcome, cause.as_deref());
    notify(notification, outcome, cause.as_deref());
    result
}

fn execute_steps(
    process: &str,
    prepared: &Prepared,
    mut issue_run: Option<&mut IssueRun<'_>>,
) -> Result<String> {
    let adapter = prepared.choice.harness.adapter();
    for worktree in &prepared.worktrees {
        skills::link_into(worktree.checkout.path(), adapter)?;
        adapter.link_instruction_fallback(worktree.checkout.path())?;
    }
    if let Some(checkout) = issue_run.as_ref().and_then(|run| run.checkout.as_ref()) {
        skills::link_into(checkout.path(), adapter)?;
        adapter.link_instruction_fallback(checkout.path())?;
    }
    let results = logs::command_log_path()
        .context("a Process has no Command log")?
        .with_extension("steps");
    fs::create_dir_all(&results)
        .with_context(|| format!("could not create {}", results.display()))?;
    let mut earlier = Vec::new();
    let mut last = String::new();
    let continuing = issue_run.as_ref().and_then(|run| {
        run.on_origin
            .then(|| (run.branch.to_string(), run.base.to_string()))
    });
    for range in group_ranges(&prepared.steps) {
        let group = &prepared.steps[range.clone()];
        if group.len() == 1 && matches!(group[0].kind, StepKind::Delivery) {
            let Some(issue_run) = issue_run.as_mut() else {
                bail!("delivery has no Issue-branch worktree");
            };
            let started = Instant::now();
            let outcome = run_delivery(prepared, &group[0], issue_run);
            finish_step(
                &group[0],
                started.elapsed(),
                outcome,
                &results,
                &mut earlier,
                &mut last,
            )?;
            continue;
        }
        let checkout = issue_run.as_ref().and_then(|run| run.checkout.as_ref());
        let outcomes = if group.len() == 1 {
            let started = Instant::now();
            let outcome = run_step(
                process,
                prepared,
                &group[0],
                &earlier,
                &results,
                range.start == 0,
                checkout,
                step_continuation(prepared, &group[0], continuing.as_ref()),
            );
            vec![(started.elapsed(), outcome)]
        } else {
            run_parallel(
                process,
                prepared,
                group,
                &earlier,
                &results,
                range.start,
                checkout,
                continuing.as_ref(),
            )
        };
        let mut group_error = None;
        for (step, (elapsed, outcome)) in group.iter().zip(outcomes) {
            match finish_step(step, elapsed, outcome, &results, &mut earlier, &mut last) {
                Ok(()) => {}
                Err(error) if group_error.is_none() => group_error = Some(error),
                Err(_) => {}
            }
        }
        if let Some(error) = group_error {
            return Err(error);
        }
    }
    Ok(last)
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
    earlier: &[(String, PathBuf)],
    results: &Path,
    group_start: usize,
    checkout: Option<&Worktree>,
    continuing: Option<&(String, String)>,
) -> Vec<(Duration, Result<String>)> {
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(group.len());
        for (offset, step) in group.iter().enumerate() {
            let cwd = step_dir(prepared, step, checkout).to_path_buf();
            let choice = prepared.choice.clone();
            let earlier = earlier.to_vec();
            let process = process.to_string();
            let results = results.to_path_buf();
            let first = group_start + offset == 0;
            let issue = prepared.issue.clone();
            let started_at = prepared.started;
            let continuation = step_continuation(prepared, step, continuing);
            handles.push(scope.spawn(move || {
                let started = Instant::now();
                let run = StepRun {
                    process: &process,
                    choice: &choice,
                    issue: issue.as_ref(),
                    started: started_at,
                    continuation,
                };
                let outcome = run_step_in(&run, &cwd, step, &earlier, &results, first);
                (started.elapsed(), outcome)
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
    outcome: Result<String>,
    results: &Path,
    earlier: &mut Vec<(String, PathBuf)>,
    last: &mut String,
) -> Result<()> {
    let duration = progress::duration(elapsed);
    match outcome {
        Ok(message) => {
            progress::step(format_args!("step {}: {duration} succeeded", step.name));
            let path = results.join(format!("{}.txt", step.name));
            fs::write(&path, &message)
                .with_context(|| format!("could not write {}", path.display()))?;
            earlier.push((step.name.clone(), path));
            *last = message;
            Ok(())
        }
        Err(error) => {
            progress::step(format_args!(
                "step {}: {duration} {}: {}",
                step.name,
                failure_outcome(),
                one_line(&error)
            ));
            if step.optional && !interrupt::requested() {
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
    earlier: &[(String, PathBuf)],
    results: &Path,
    first: bool,
    checkout: Option<&Worktree>,
    continuation: Option<(String, String)>,
) -> Result<String> {
    let run = StepRun {
        process,
        choice: &prepared.choice,
        issue: prepared.issue.as_ref(),
        started: prepared.started,
        continuation,
    };
    run_step_in(
        &run,
        step_dir(prepared, step, checkout),
        step,
        earlier,
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

fn run_delivery(
    prepared: &Prepared,
    step: &ResolvedStep,
    issue_run: &mut IssueRun<'_>,
) -> Result<String> {
    let checkout = issue_run
        .checkout
        .take()
        .context("the Issue-branch worktree is gone")?;
    progress::step(format_args!(
        "running step {} with {choice}",
        step.name,
        choice = prepared.choice
    ));
    let mut pull_request = PullRequest::new(
        issue_run.issue,
        issue_run.branch,
        issue_run.base,
        GitHub::new(),
    );
    let child_asks = ChildAsks {
        harness: prepared.choice.clone(),
        ..ChildAsks::default()
    };
    let started = prepared.started;
    match (delivery::Delivery {
        security_review: false,
        child_asks: &child_asks,
        issue: issue_run.issue,
        base: issue_run.base,
        goal: Goal::ReadyForReview,
        base_fix: issue_run.base_fix,
        logs: issue_run.logs,
        started,
    })
    .publish(checkout, &mut pull_request)
    {
        Ok(url) => Ok(url),
        Err(mut failed) => {
            if let Some(recovery) = &mut failed.recovery {
                recovery.add_command_context(continuation(issue_run), issue_run.on_origin);
                for line in recovery.lines() {
                    progress::step(line);
                }
            }
            Err(failed.error)
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
    let preservation = recovery::preservation_of(preserved, |line| progress::step(line));
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
    earlier: &[(String, PathBuf)],
    results: &Path,
    first: bool,
) -> Result<String> {
    crate::resources::wait()?;
    match &step.kind {
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
                earlier,
                first,
                run.issue,
                run.continuation
                    .as_ref()
                    .map(|(branch, base)| prompt::ProcessContinuation { branch, base }),
            );
            match run.issue {
                Some(issue) => {
                    run_skill_on_issue(cwd, run.choice, &step.name, &prompt, issue, run.started)
                }
                None => {
                    skill::run_session(cwd, run.choice, &format!("Step {}", step.name), &prompt)
                }
            }
        }
        StepKind::Delivery => bail!("delivery cannot be in a parallel group"),
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
    match trust::untrusted_edit(&crate::github::GitHub::new(), issue, Some(started))? {
        Some(text) => bail!(
            "#{number} {text} edited since the Process started by an account that is not a Trusted account; starting no further session",
            number = issue.number,
        ),
        None => delivery::hand_over_issue(issue, worktree),
    }
}

/// A session Step on an issue: the trust check and issue file before the
/// session, and again before its one Resume.
fn run_skill_on_issue(
    worktree: &Path,
    choice: &Choice,
    step: &str,
    prompt: &str,
    issue: &IssueUrl,
    started: DateTime<Utc>,
) -> Result<String> {
    let label = format!("Step {step}");
    prepare_for_session(issue, started, worktree)?;
    let ended = skill::run_session_ending(worktree, choice, None, prompt)?;
    let (ended, ended_last) = if ended.killed.is_empty() {
        (ended, "session")
    } else if let Some(session_id) = ended.session_id.clone() {
        progress::step(format_args!(
            "{label}: background work was killed as the session ended; resuming it once"
        ));
        prepare_for_session(issue, started, worktree)?;
        let resume_prompt = prompt::resume(&ended.killed_work());
        let resumed =
            skill::run_session_ending(worktree, choice, Some(&session_id), &resume_prompt)?;
        (resumed, "Resume")
    } else {
        (ended, "session")
    };
    if !ended.killed.is_empty()
        && let Some(message) = ended
            .final_message
            .clone()
            .filter(|text| !text.trim().is_empty())
    {
        let ending = session::ending_with(&ended.killed_work());
        progress::step(format_args!(
            "{label}: the {ended_last} {ending}; carrying on, as it may have been abandoned"
        ));
        return Ok(message);
    }
    ended.require_final_message(&label)
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
    let named = !prepared.declared.is_empty();
    for range in group_ranges(&prepared.steps) {
        let names: Vec<_> = prepared.steps[range.clone()]
            .iter()
            .map(|step| step.name.as_str())
            .collect();
        for step in &prepared.steps[range.clone()] {
            let _ = writeln!(text, "{}", step_plan(step, named));
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
    let path = process_path(name)?;
    refuse_reserved(&path, name)?;
    let text = read_process(&path)?;
    let file = parse(&path, &text)?;
    refuse_worktrees(&path, &file)?;
    refuse_issue_branch(&path, &file)?;
    Ok((path, file))
}

fn prepare_from(
    path: &Path,
    config: &UserConfig,
    file: ProcessFile,
    issue: Option<IssueUrl>,
    started: DateTime<Utc>,
    launch: &Git,
    commit: &str,
    issue_checkout: Option<&Path>,
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
    let skip = issue_checkout.and(issue_worktree.as_deref());
    let worktrees = create_worktrees(launch, commit, &file, skip)?;
    let mut steps = Vec::new();
    for step in &file.steps {
        let checkout = resolve_dir(&worktrees, step, issue_worktree.as_deref(), issue_checkout);
        steps.push(resolve(path, step, checkout)?);
    }
    let mut choice = Choice::of(&Asked::default(), &config.harness);
    choice.check_without_probe()?;
    Ok(Prepared {
        worktrees,
        declared,
        issue_worktree,
        _written: WrittenGuard,
        choice,
        description: file.description,
        steps,
        issue,
        started,
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

fn refuse_issue_branch(path: &Path, file: &ProcessFile) -> Result<()> {
    let mut issue_branches = file
        .worktrees
        .iter()
        .filter(|worktree| worktree.issue_branch_line.is_some());
    let Some(issue_branch) = issue_branches.next() else {
        for step in &file.steps {
            if let DraftKind::Catalogue { line } = &step.kind {
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
        if let DraftKind::Catalogue { line } = &step.kind {
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

fn resolve(path: &Path, step: &StepDraft, worktree: &Path) -> Result<ResolvedStep> {
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
        DraftKind::Catalogue { .. } => StepKind::Delivery,
    };
    Ok(ResolvedStep {
        name: step.name.clone(),
        optional: step.optional,
        worktree: step.worktree.as_ref().map(|(name, _)| name.clone()),
        parallel: step.parallel.as_ref().map(|(name, _)| name.clone()),
        kind,
    })
}

fn read_process(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == ErrorKind::NotFound => Err(rule(
            path,
            1,
            "missing file",
            "the process file does not exist",
        )),
        Err(error) => Err(rule(
            path,
            1,
            "file",
            format!("could not read the file: {error}"),
        )),
    }
}

fn refuse_reserved(path: &Path, name: &str) -> Result<()> {
    if reserved(name) {
        Err(rule(
            path,
            1,
            "reserved name",
            format!("{name} is reserved"),
        ))
    } else {
        Ok(())
    }
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
    })
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
        if !single_component(key) {
            return Err(rule(
                path,
                name_line,
                "worktree name",
                "a worktree name must be a single path component",
            ));
        }
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
    if value.chars().any(|c| c == '\n' || c == '\r') {
        return Err(rule(
            path,
            line,
            "description",
            "description must be one line",
        ));
    }
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
            "catalogue" => {
                let line = step_catalogue(path, line, item)?;
                kinds.push((DraftKind::Catalogue { line }, line));
            }
            "worktree" => worktree = Some(step_worktree(path, line, item)?),
            "parallel" => parallel = Some(step_parallel(path, line, item)?),
            "optional" => optional = step_optional(path, line, item)?,
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
    Ok(StepDraft {
        name,
        name_line,
        optional,
        worktree,
        parallel,
        kind,
    })
}

fn step_worktree(path: &Path, line: usize, item: &Item) -> Result<(String, usize)> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "worktree", "worktree must be a string"));
    };
    if !single_component(value) {
        return Err(rule(
            path,
            line,
            "worktree",
            "worktree must be a single path component",
        ));
    }
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
    if !single_component(value) {
        return Err(rule(
            path,
            line,
            "step name",
            "name must be a single path component",
        ));
    }
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
    Ok((value.to_string(), line))
}

fn step_catalogue(path: &Path, line: usize, item: &Item) -> Result<usize> {
    let Some(value) = item.as_str() else {
        return Err(rule(path, line, "catalogue", "catalogue must be a string"));
    };
    if value != "delivery" {
        return Err(rule(path, line, "catalogue", "catalogue must be delivery"));
    }
    Ok(line)
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
    Ok((value.to_string(), line))
}

fn processes_dir() -> Result<PathBuf> {
    Ok(crate::config::home()?.join(".thirdshift/processes"))
}

fn process_path(name: &str) -> Result<PathBuf> {
    Ok(processes_dir()?.join(format!("{name}.toml")))
}

fn reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

fn single_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0', '\n', '\r'])
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
