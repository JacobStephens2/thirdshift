//! An Architect run up to its plan: an Architecture review of the Base
//! branch, the one its command named or else the one checked out in the
//! Launch directory, from the Launch directory with no Issue URL, then the
//! checks on the Architect plan it published and the label change that marks
//! it ready and labels it `architect-plan`, for the command to stop at or to
//! dispatch. A review that found no Strong candidate published no plan, and
//! the Architect run ends on the issue it named instead: the idea issue it
//! filed for its top recommendation, or the open issue that already covers
//! it. Only one Architect run per repository runs at a time on a machine: one
//! started while another is still running is skipped, before any review. So
//! is one that finds an Architect plan still open on the repository: it never
//! retries or dispatches an Architect plan that is already there.

use std::fmt;
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};

use crate::config;
use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::plugin::Plugin;
use crate::preflight;
use crate::progress;
use crate::prompt;
use crate::run;
use crate::session::{Logs, Sessions};
use crate::spec_run::{self, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::worktree::ReviewWorktree;

/// The Architecture review session's kind, in its progress lines and log
/// name.
const REVIEW: &str = "architecture-review";

/// The label thirdshift marks an Architect plan with, which a later Architect
/// run finds an open one by.
const ARCHITECT_PLAN_LABEL: &str = "architect-plan";

/// The description the `architect-plan` label is added to the repository
/// with if the repository lacks it.
const ARCHITECT_PLAN_DESCRIPTION: &str =
    "An Architect plan: the Spec or Ticket an Architecture review published";

/// How an Architect run ended, short of a failure and before any dispatch.
/// Its `Display` is the line that says how it ended.
#[derive(Debug)]
pub enum Outcome {
    /// Skipped, before any review, having done nothing.
    Skipped(Skipped),
    /// Its Architecture review ended on an issue.
    Reviewed(Reviewed),
}

impl Outcome {
    /// How the Architect run ended, as its Run notification's subject says
    /// it when nothing was dispatched.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Skipped(_) => "skipped",
            Self::Reviewed(reviewed) => reviewed.review(),
        }
    }

    /// The URLs of the issues the Architect run ended on, as its stdout
    /// carries them when nothing was dispatched: the issue its Architecture
    /// review ended on, or each open Architect plan it was skipped for. None
    /// when it was skipped as another is still running.
    pub fn urls(&self) -> Vec<&str> {
        match self {
            Self::Skipped(Skipped::AlreadyRunning(_)) => Vec::new(),
            Self::Skipped(Skipped::OpenPlans(plans)) => {
                plans.iter().map(|(plan, _)| plan.url.as_str()).collect()
            }
            Self::Reviewed(reviewed) => vec![reviewed.url()],
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Skipped(skipped) => skipped.fmt(f),
            Self::Reviewed(reviewed) => reviewed.fmt(f),
        }
    }
}

/// Why an Architect run was skipped. Its `Display` is the reason, as the
/// skipped run's progress line and its Run notification give it.
#[derive(Debug)]
pub enum Skipped {
    /// Another Architect run on this repository is still running on this
    /// machine, the Spec run or Run it dispatched included.
    AlreadyRunning(Repo),
    /// These Architect plans, each with its title, are still open on this
    /// repository: at least one.
    OpenPlans(Vec<(IssueUrl, String)>),
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(repo) => {
                write!(f, "an Architect run is already running on {}", repo.slug())
            }
            Self::OpenPlans(plans) => {
                let still_open: Vec<String> = plans
                    .iter()
                    .map(|(plan, title)| {
                        format!(
                            "Architect plan #{} \"{title}\" is still open: pick it up with thirdshift {}",
                            plan.number, plan.url
                        )
                    })
                    .collect();
                f.write_str(&still_open.join("; "))
            }
        }
    }
}

/// How an Architecture review ended, short of a failure, with the issue it
/// ended on. Its `Display` is the line that says how it ended, naming that
/// issue.
#[derive(Debug)]
pub enum Reviewed {
    /// The plan the review published is marked ready, with the Architect
    /// run's Base branch, which the run the plan is dispatched as takes.
    PlanReady { plan: IssueUrl, base: String },
    /// The review found no Strong candidate, and filed its top
    /// recommendation as this issue.
    IdeaFiled(IssueUrl),
    /// The review found no Strong candidate, and filed nothing: this open
    /// issue already covers its top recommendation.
    AlreadyFiled(IssueUrl),
}

impl Reviewed {
    /// How the Architecture review ended, as the Architect run's Run
    /// notification says it.
    pub fn review(&self) -> &'static str {
        match self {
            Self::PlanReady { .. } => "plan published",
            Self::IdeaFiled(_) => "idea filed",
            Self::AlreadyFiled(_) => "idea already filed",
        }
    }

    /// The URL of the issue the Architecture review ended on.
    pub fn url(&self) -> &str {
        match self {
            Self::PlanReady { plan: issue, .. }
            | Self::IdeaFiled(issue)
            | Self::AlreadyFiled(issue) => &issue.url,
        }
    }
}

impl fmt::Display for Reviewed {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let url = self.url();
        match self {
            Self::PlanReady { .. } => write!(f, "plan {url} is ready for an agent"),
            Self::IdeaFiled(_) => write!(
                f,
                "no Strong candidate: the Architecture review filed the idea {url}"
            ),
            Self::AlreadyFiled(_) => write!(
                f,
                "no Strong candidate: {url} already covers the Architecture review's top recommendation, so it filed nothing"
            ),
        }
    }
}

/// Run an Architecture review of the Base branch, pointed at `focus` if
/// given, and mark the plan it publishes ready. The Base branch is `base`,
/// the branch the command named, whatever the Launch directory has checked
/// out, or without one the branch checked out there. A review that reports
/// an idea issue it filed, or the open issue that already covers its top
/// recommendation, instead of a plan, changes no label. With `launch_pull`,
/// the Launch directory's checkout of the Base branch, if that is the branch
/// checked out, is first brought up to date with origin. The review's
/// worktree and the plugin directory are gone when this returns. A failure
/// after the plan is published leaves its labels as the review left them.
///
/// Once the preflight checks pass, and before anything else, the Architect
/// run is skipped, with nothing done, if another on the same repository is
/// still running on this machine. Otherwise this process is that repository's
/// one Architect run until it exits, through whatever it dispatches. It is
/// then skipped, likewise, if the repository has an open Architect plan: only
/// then, so that the plan of an Architect run still running is never taken
/// for an unfinished one.
pub fn run(
    focus: Option<&str>,
    base: Option<&str>,
    logs_dir: &Path,
    launch_pull: bool,
) -> Result<Outcome, FailedRun> {
    let started = Utc::now();
    let timestamp = started.format("%Y%m%dT%H%M%SZ").to_string();
    let (launch, origin, repo) = launch()?;
    preflight::check_identity(&launch)?;
    let checked_out = launch
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok();
    let base = base
        .or(checked_out.as_deref())
        .context(
            "HEAD is detached; check out the branch the Architecture review should scan, \
             or name it with base <branch>",
        )?
        .to_string();
    preflight::check_base_branch(&launch, &base)?;
    let Some(lock) = try_run_lock(&repo)? else {
        return Ok(Outcome::Skipped(Skipped::AlreadyRunning(repo)));
    };
    // Never closed, so the lock is held for as long as this process lives,
    // through the Spec run or Run its plan is dispatched as, and the
    // operating system releases it however the process ends.
    std::mem::forget(lock);
    let open_plans = github::open_issues_labelled(&repo.slug(), ARCHITECT_PLAN_LABEL)?;
    if !open_plans.is_empty() {
        return Ok(Outcome::Skipped(Skipped::OpenPlans(open_plans)));
    }
    if launch_pull {
        run::pull_base_branch(&launch, checked_out.as_deref(), &base);
    }

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let worktree = ReviewWorktree::create(&launch, &repo.name, &base)?;
    let logs = Logs::of_architect_run(&repo, logs_dir, &timestamp);
    let mut log = logs.path(REVIEW);
    review(worktree, &base, focus, &logs, &mut log)
        .and_then(|final_message| conclude(final_message.as_deref(), &origin, started, base))
        .map(Outcome::Reviewed)
        .map_err(|error| FailedRun {
            log: log.exists().then_some(log),
            ..FailedRun::from(error)
        })
}

/// The Launch directory, the URL of its `origin`, and the GitHub repository
/// that names: an Architect run's, as it has no Issue URL to name one.
fn launch() -> Result<(Git, String, Repo)> {
    let launch = Git::new(std::env::current_dir().context("no current directory")?);
    let origin = launch.run(&["config", "remote.origin.url"])?;
    let repo = Repo::of_origin(&origin)
        .with_context(|| format!("origin {origin} is not a GitHub repository"))?;
    Ok((launch, origin, repo))
}

/// The repository an Architect run started from the Launch directory is on.
pub fn repo() -> Result<Repo> {
    launch().map(|(_, _, repo)| repo)
}

/// Try, without waiting, for the lock that makes its holder the one Architect
/// run on `repo` on this machine: an advisory lock on a file under
/// `~/.thirdshift` named for the repository, as GitHub compares names,
/// whatever their case. It is held until the file returned is closed, or the
/// process ends. `None` if another process holds it: an Architect run on
/// `repo` is still running. The lock file itself is never deleted, and means
/// nothing unless locked.
fn try_run_lock(repo: &Repo) -> Result<Option<File>> {
    let dir = config::home()?
        .join(".thirdshift/architect-locks")
        .join(repo.owner.to_ascii_lowercase());
    fs::create_dir_all(&dir).with_context(|| format!("can't create {}", dir.display()))?;
    let path = dir.join(format!("{}.lock", repo.name.to_ascii_lowercase()));
    let file = File::create(&path).with_context(|| format!("can't open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("can't lock {}", path.display()))
        }
    }
}

/// The Architecture review session in `worktree`, which is removed once the
/// session ends. Returns the session's final message. `log` is left at the
/// most recent session's log.
fn review(
    worktree: ReviewWorktree,
    base: &str,
    focus: Option<&str>,
    logs: &Logs,
    log: &mut PathBuf,
) -> Result<Option<String>> {
    let plugin = Plugin::write()?;
    let sessions = Sessions {
        logs,
        worktree: worktree.path(),
        plugin_dir: plugin.path(),
    };
    match focus {
        Some(focus) => progress::step(format_args!(
            "starting the Architecture review of {base}, focused on: {focus}"
        )),
        None => progress::step(format_args!("starting the Architecture review of {base}")),
    }
    sessions.run_to_final_message(REVIEW, &prompt::architecture_review(base, focus), log)
}

/// What an Architecture review reported in the last line of its final
/// message.
#[derive(Debug, PartialEq, Eq)]
enum Report {
    /// The plan it published: a Spec, or a single Ticket.
    Plan(IssueUrl),
    /// The issue it filed for a top recommendation that is not Strong.
    Idea(IssueUrl),
    /// The open issue that already covers that recommendation.
    AlreadyFiled(IssueUrl),
}

impl Report {
    /// Read the last line of `final_message` that isn't blank. `None` unless
    /// it is one of the lines the Architecture review prompt asks for, with
    /// an Issue URL and nothing else after it.
    fn read(final_message: &str) -> Option<Self> {
        let line = final_message
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())?;
        let naming = |start: &str, kind: fn(IssueUrl) -> Report| {
            let url = line.strip_prefix(start)?;
            IssueUrl::parse(url).ok().map(kind)
        };
        naming(prompt::PLAN_LINE, Report::Plan)
            .or_else(|| naming(prompt::IDEA_LINE, Report::Idea))
            .or_else(|| naming(prompt::ALREADY_FILED_LINE, Report::AlreadyFiled))
    }
}

/// End the Architecture review, of the Base branch `base`, on the issue the
/// last line of its `final_message` names: a plan is marked ready, and an
/// idea issue, or the issue that already covers the top recommendation, is
/// left as it is. Fails if the session had no final message, if its last line
/// is not one the prompt asks for, or if the plan can't be marked ready.
fn conclude(
    final_message: Option<&str>,
    origin: &str,
    started: DateTime<Utc>,
    base: String,
) -> Result<Reviewed> {
    match final_message.and_then(Report::read) {
        Some(Report::Plan(plan)) => {
            mark_plan_ready(&plan, origin, started)?;
            Ok(Reviewed::PlanReady { plan, base })
        }
        Some(Report::Idea(idea)) => Ok(Reviewed::IdeaFiled(idea)),
        Some(Report::AlreadyFiled(issue)) => Ok(Reviewed::AlreadyFiled(issue)),
        None => bail!("the Architecture review ended without the final line its prompt asks for"),
    }
}

/// Mark `plan` ready: check it, then swap its `needs-triage` for
/// `ready-for-agent` and label it `architect-plan`, in one request, having
/// added that label to the repository if it lacks it. Fails, changing no
/// label, if the plan fails its checks.
fn mark_plan_ready(plan: &IssueUrl, origin: &str, started: DateTime<Utc>) -> Result<()> {
    progress::step(format_args!(
        "the Architecture review published the plan {}",
        plan.url
    ));
    let kept = labels_to_keep(plan, origin, started)?;
    if interrupt::requested() {
        bail!("interrupted");
    }
    progress::step(format_args!(
        "marking the plan ready: swapping {NEEDS_TRIAGE} for {READY_FOR_AGENT} and adding {ARCHITECT_PLAN_LABEL} on #{}",
        plan.number
    ));
    github::ensure_labels(
        &plan.repo_slug(),
        &[(ARCHITECT_PLAN_LABEL, ARCHITECT_PLAN_DESCRIPTION)],
    )?;
    let added = [READY_FOR_AGENT, ARCHITECT_PLAN_LABEL];
    // GitHub's label names are case-insensitive.
    let mut labels: Vec<&str> = kept
        .iter()
        .map(String::as_str)
        .filter(|kept| !added.iter().any(|added| added.eq_ignore_ascii_case(kept)))
        .collect();
    labels.extend(added);
    github::set_labels(plan, &labels)
}

/// The labels `plan` keeps once it is marked ready: all it has but
/// `needs-triage`. Fails unless the plan passes its checks: it is in the
/// repository at `origin`, open, created since the Architect run `started`,
/// and has no other label that makes an Unready Ticket.
fn labels_to_keep(plan: &IssueUrl, origin: &str, started: DateTime<Utc>) -> Result<Vec<String>> {
    if !plan.matches_origin(origin) {
        bail!(
            "the plan {} is not in the repository at origin {origin}",
            plan.url
        );
    }
    let issue = github::issue(plan)?;
    if !issue.is_open {
        bail!("the plan {} is closed", plan.url);
    }
    // GitHub's times are to the second.
    if issue.created.timestamp() < started.timestamp() {
        bail!(
            "the plan {} was created before this Architect run started",
            plan.url
        );
    }
    let mut kept = issue.labels;
    kept.retain(|label| label != NEEDS_TRIAGE);
    if let Some(unready) = spec_run::unready_label(&kept) {
        bail!("the plan {} is labelled {unready}", plan.url);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://github.com/acme/widgets/issues/8";

    fn issue() -> IssueUrl {
        IssueUrl::parse(URL).unwrap()
    }

    #[test]
    fn the_report_is_the_last_line_whichever_of_the_three_it_is() {
        for (line, report) in [
            ("Architecture review plan: ", Report::Plan(issue())),
            ("Architecture review idea: ", Report::Idea(issue())),
            (
                "Architecture review already filed: ",
                Report::AlreadyFiled(issue()),
            ),
        ] {
            let message = format!("Reviewed the codebase.\n\n{line}{URL}\n\n");
            assert_eq!(Report::read(&message), Some(report), "{line}");
        }
    }

    #[test]
    fn a_final_message_without_the_line_as_asked_for_reports_nothing() {
        for message in [
            "",
            "Published the plan.",
            "Architecture review plan: https://github.com/acme/widgets/pull/8",
            "Architecture review plan: #8",
            "Architecture review plan: https://github.com/acme/widgets/issues/8 (a Spec)",
            "The plan: https://github.com/acme/widgets/issues/8",
            "Architecture review plan: https://github.com/acme/widgets/issues/8\nThat's all.",
        ] {
            assert_eq!(Report::read(message), None, "{message:?}");
        }
    }
}
