//! An Architect run up to its plan: an Architecture review of the Base
//! branch, the one its command named or else the one checked out in the
//! Launch directory, from the Launch directory with no Issue URL, then the
//! checks on the Architect plan it published and the label change that marks
//! it ready and labels it `architect-plan`, for the command to stop at or to
//! dispatch. A review that found no Strong candidate published no plan, and
//! the Architect run ends on the issue it named instead, labelled an
//! Architect idea: the idea issue it filed for its top recommendation, or the
//! open issue that already covers it. Only one Architect run or Pickup run
//! per repository runs at a time on a machine: an Architect run started while
//! another of either is still running is skipped, before any review. So is
//! one that finds an Architect plan still open on the repository: it never
//! retries or dispatches an Architect plan that is already there. And so is
//! one that finds an Architect idea there waiting for triage: the factory has
//! run out of Strong ideas until the Day shift decides on it. And so, last,
//! is one that finds a Ready issue there, by the Pickup run's own search:
//! work a human shaped goes first.

use std::fmt;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};

use crate::failed_run::FailedRun;
use crate::github::{self, ListedIssue};
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Label, Labels, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::progress;
use crate::prompt;
use crate::ready::{self, ReadyIssue};
use crate::run;
use crate::session::{Logs, Sessions};
use crate::worktree::ReviewWorktree;

/// The Architecture review session's kind, in its progress lines and log
/// name.
const REVIEW: &str = "architecture-review";

/// The label thirdshift marks an Architect plan with, which a later Architect
/// run finds an open one by.
const ARCHITECT_PLAN: Label = Label::new(
    "architect-plan",
    "An Architect plan: the Spec or Ticket an Architecture review published",
);

/// The label thirdshift marks an Architect idea with, which, with
/// `needs-triage`, a later Architect run finds one waiting for triage by.
const ARCHITECT_IDEA: Label = Label::new(
    "architect-idea",
    "An Architect idea: the issue an Architecture review with no Strong candidate ended on",
);

/// How an Architect run ended, short of a failure and before any dispatch.
#[derive(Debug)]
pub enum Outcome {
    /// Skipped, before any review, having done nothing.
    Skipped(Skipped),
    /// Its Architecture review ended on an issue.
    Reviewed(Reviewed),
}

/// Why an Architect run was skipped. Its `Display` is the reason, as the
/// skipped run's progress line gives it.
#[derive(Debug)]
pub enum Skipped {
    /// Another Architect run on this repository, or a Pickup run, is still
    /// running on this machine, the Spec run or Run it dispatched included.
    AlreadyRunning(AlreadyRunning),
    /// These Architect plans are still open on this repository: at least
    /// one.
    OpenPlans(Vec<ListedIssue>),
    /// These Architect ideas are open on this repository and still labelled
    /// `needs-triage`: at least one.
    IdeasWaiting(Vec<ListedIssue>),
    /// This Ready issue, the lowest-numbered on this repository, goes first.
    ReadyIssue(ListedIssue),
}

impl Skipped {
    /// The URLs of the issues the Architect run was skipped for, as its
    /// stdout carries them: each open Architect plan, or each Architect idea
    /// waiting for triage, or the Ready issue. None when it was skipped as
    /// another, or a Pickup run, is still running.
    pub fn urls(&self) -> Vec<&str> {
        match self {
            Self::AlreadyRunning(_) => Vec::new(),
            Self::OpenPlans(issues) | Self::IdeasWaiting(issues) => issues
                .iter()
                .map(|listed| listed.issue.url.as_str())
                .collect(),
            Self::ReadyIssue(listed) => vec![listed.issue.url.as_str()],
        }
    }
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(running) => running.fmt(f),
            Self::OpenPlans(plans) => write_joined(f, plans, |plan, title| {
                format!(
                    "Architect plan #{} \"{title}\" is still open: pick it up with thirdshift {}",
                    plan.number, plan.url
                )
            }),
            Self::IdeasWaiting(ideas) => write_joined(f, ideas, |idea, title| {
                format!(
                    "Architect idea #{} \"{title}\" is waiting for triage: {}",
                    idea.number, idea.url
                )
            }),
            Self::ReadyIssue(ListedIssue { issue, title, .. }) => write!(
                f,
                "Ready issue #{} \"{title}\" goes first: {}",
                issue.number, issue.url
            ),
        }
    }
}

/// Write what `says` of each of `issues`, with its title, joined by `; `.
fn write_joined(
    f: &mut fmt::Formatter,
    issues: &[ListedIssue],
    says: impl Fn(&IssueUrl, &str) -> String,
) -> fmt::Result {
    let said: Vec<String> = issues
        .iter()
        .map(|listed| says(&listed.issue, &listed.title))
        .collect();
    f.write_str(&said.join("; "))
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
/// recommendation, instead of a plan, has that issue labelled an Architect
/// idea. With `launch_pull`, the Launch directory's checkout of the Base
/// branch, if that is the branch checked out, is first brought up to date
/// with origin. The review's
/// worktree and the plugin directory are gone when this returns. A failure
/// after the plan is published leaves its labels as the review left them.
///
/// Once the preflight checks pass, and before anything else, the Architect
/// run is skipped, with nothing done, if another on the same repository, or a
/// Pickup run, is still running on this machine: see [`launch::start`]. It is
/// then skipped, likewise, if the repository has an open Architect plan: only
/// then, so that the plan of an Architect run still running is never taken
/// for an unfinished one. It is then skipped if the repository has an open
/// Architect idea still labelled `needs-triage`. It is then skipped if the
/// repository has a Ready issue, by the very search a Pickup run makes, with
/// its lines on the issues passed over, but with no Sweep and no Claim limit,
/// which are the Pickup run's. A skip is recorded in the repository's
/// Activity log. Past all four, it records that it started work, which keeps
/// its Command log with its repository's logs, where its Session logs go too.
pub fn run(
    focus: Option<&str>,
    base: Option<&str>,
    launch_pull: bool,
) -> Result<Outcome, FailedRun> {
    let started = Utc::now();
    let Launch {
        git: launch,
        origin,
        repo,
        checked_out,
        base,
    } = match launch::start(base)? {
        Start::Clear(launch) => launch,
        Start::AlreadyRunning(running) => {
            let repo = &running.0;
            logs::skipped(Pass::ArchitectRun, repo, &running);
            return Ok(Outcome::Skipped(Skipped::AlreadyRunning(running)));
        }
    };
    let open_plans = github::open_issues_labelled(&repo.slug(), ARCHITECT_PLAN)?;
    if !open_plans.is_empty() {
        return Ok(skip(&repo, Skipped::OpenPlans(open_plans)));
    }
    let ideas = github::open_issues_labelled(&repo.slug(), ARCHITECT_IDEA)?;
    let waiting: Vec<_> = ideas
        .into_iter()
        .filter(|idea| idea.labels.has(NEEDS_TRIAGE))
        .collect();
    if !waiting.is_empty() {
        return Ok(skip(&repo, Skipped::IdeasWaiting(waiting)));
    }
    if let Some(ReadyIssue { listed, .. }) = ready::first(&launch, &repo)? {
        return Ok(skip(&repo, Skipped::ReadyIssue(listed)));
    }
    logs::started(Work::ArchitectRun(&repo));
    if launch_pull {
        run::pull_base_branch(&launch, checked_out.as_deref(), &base);
    }

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let worktree = ReviewWorktree::create(&launch, &repo.name, &base)?;
    let logs = Logs::of_architect_run(&repo);
    let (reviewed, log) = review(worktree, &base, focus, &origin, started, &logs);
    reviewed.map(Outcome::Reviewed).map_err(|error| FailedRun {
        log,
        ..FailedRun::from(error)
    })
}

/// The Architect run skipped as `skipped` says, recorded in the Activity log
/// of `repo`.
fn skip(repo: &Repo, skipped: Skipped) -> Outcome {
    logs::skipped(Pass::ArchitectRun, repo, &skipped);
    Outcome::Skipped(skipped)
}

/// The Architecture review session in `worktree`, of the Base branch `base`,
/// then [`conclude`] on its final message. The worktree is removed once the
/// review is concluded. Returns how it ended with the most recent session's
/// log, if a session created it.
fn review(
    worktree: ReviewWorktree,
    base: &str,
    focus: Option<&str>,
    origin: &str,
    started: DateTime<Utc>,
    logs: &Logs,
) -> (Result<Reviewed>, Option<PathBuf>) {
    Sessions::within(logs, worktree.path(), |sessions| {
        match focus {
            Some(focus) => progress::step(format_args!(
                "starting the Architecture review of {base}, focused on: {focus}"
            )),
            None => progress::step(format_args!("starting the Architecture review of {base}")),
        }
        let prompt = prompt::architecture_review(base, focus);
        let final_message = sessions.run_to_final_message(REVIEW, &prompt)?;
        conclude(final_message.as_deref(), origin, started, base)
    })
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
/// labelled an Architect idea. Fails if the session had no final message, if
/// its last line is not one the prompt asks for, or if the plan can't be
/// marked ready or the idea labelled.
fn conclude(
    final_message: Option<&str>,
    origin: &str,
    started: DateTime<Utc>,
    base: &str,
) -> Result<Reviewed> {
    match final_message.and_then(Report::read) {
        Some(Report::Plan(plan)) => {
            mark_plan_ready(&plan, origin, started)?;
            let base = base.to_string();
            Ok(Reviewed::PlanReady { plan, base })
        }
        Some(Report::Idea(idea)) => {
            label_idea(&idea)?;
            Ok(Reviewed::IdeaFiled(idea))
        }
        Some(Report::AlreadyFiled(issue)) => {
            label_idea(&issue)?;
            Ok(Reviewed::AlreadyFiled(issue))
        }
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
        "marking the plan ready: swapping {NEEDS_TRIAGE} for {READY_FOR_AGENT} and adding {ARCHITECT_PLAN} on #{}",
        plan.number
    ));
    github::ensure_labels(&plan.repo_slug(), &[ARCHITECT_PLAN])?;
    github::set_labels(plan, &kept.swapped(&[], &[READY_FOR_AGENT, ARCHITECT_PLAN]))
}

/// Label `idea` an Architect idea: put `needs-triage` and `architect-idea`
/// on it, in one request that keeps its other labels, having added
/// `architect-idea` to the repository if it lacks it. `needs-triage` goes
/// back on an issue that had been triaged: the factory again takes it for the
/// best next move.
fn label_idea(idea: &IssueUrl) -> Result<()> {
    if interrupt::requested() {
        bail!("interrupted");
    }
    progress::step(format_args!(
        "labelling #{} an Architect idea: adding {NEEDS_TRIAGE} and {ARCHITECT_IDEA}",
        idea.number
    ));
    let label = || -> Result<()> {
        github::ensure_labels(&idea.repo_slug(), &[ARCHITECT_IDEA])?;
        let labels = github::issue_labels(idea)?;
        github::set_labels(idea, &labels.swapped(&[], &[NEEDS_TRIAGE, ARCHITECT_IDEA]))
    };
    label().with_context(|| format!("could not label the Architect idea #{}", idea.number))
}

/// The labels `plan` keeps once it is marked ready: all it has but
/// `needs-triage`. Fails unless the plan passes its checks: it is in the
/// repository at `origin`, open, created since the Architect run `started`,
/// and has no other label that makes an Unready Ticket.
fn labels_to_keep(plan: &IssueUrl, origin: &str, started: DateTime<Utc>) -> Result<Labels> {
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
    let kept = issue.labels.swapped(&[NEEDS_TRIAGE], &[]);
    if let Some(unready) = kept.unready() {
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
