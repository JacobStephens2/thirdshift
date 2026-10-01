//! One Architect run: an Architecture review of the Base branch, from the
//! Launch directory with no Issue URL, then the checks on the plan it
//! published and the label swap that marks the plan ready. A review that
//! found no Strong candidate published no plan, and the Architect run ends
//! on the issue it named instead: the idea issue it filed for its top
//! recommendation, or the open issue that already covers it.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};

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
use crate::spec_run::{self, NEEDS_TRIAGE};
use crate::worktree::ReviewWorktree;

/// The Architecture review session's kind, in its progress lines and log
/// name.
const REVIEW: &str = "architecture-review";

/// The triage label thirdshift swaps the plan's `needs-triage` for once the
/// plan passes its checks.
const READY_FOR_AGENT: &str = "ready-for-agent";

/// How an Architect run ended, short of a failure, with the issue it ended
/// on. Its `Display` is the line that says how it ended, naming that issue.
#[derive(Debug)]
pub enum Outcome {
    /// The plan the review published is marked ready.
    PlanReady(IssueUrl),
    /// The review found no Strong candidate, and filed its top
    /// recommendation as this issue.
    IdeaFiled(IssueUrl),
    /// The review found no Strong candidate, and filed nothing: this open
    /// issue already covers its top recommendation.
    AlreadyFiled(IssueUrl),
}

impl Outcome {
    /// The URL of the issue the Architect run ended on.
    pub fn url(&self) -> &str {
        match self {
            Self::PlanReady(issue) | Self::IdeaFiled(issue) | Self::AlreadyFiled(issue) => {
                &issue.url
            }
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let url = self.url();
        match self {
            Self::PlanReady(_) => write!(f, "plan {url} is ready for an agent"),
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

/// Run an Architecture review of the branch checked out in the Launch
/// directory, the Base branch, pointed at `focus` if given, and mark the plan
/// it publishes ready. A review that reports an idea issue it filed, or the
/// open issue that already covers its top recommendation, instead of a plan,
/// changes no label. With `launch_pull`, the Launch directory's checkout of
/// the Base branch is first brought up to date with origin. The review's
/// worktree and the plugin directory are gone when this returns. A failure
/// after the plan is published leaves its labels as the review left them.
pub fn run(focus: Option<&str>, logs_dir: &Path, launch_pull: bool) -> Result<Outcome, FailedRun> {
    let started = Utc::now();
    let timestamp = started.format("%Y%m%dT%H%M%SZ").to_string();
    let launch = Git::new(std::env::current_dir().context("no current directory")?);

    let origin = launch.run(&["config", "remote.origin.url"])?;
    let repo = Repo::of_origin(&origin)
        .with_context(|| format!("origin {origin} is not a GitHub repository"))?;
    preflight::check_identity(&launch)?;
    let base = launch
        .run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .context("HEAD is detached; check out the branch the Architecture review should scan")?;
    preflight::check_base_branch(&launch, &base)?;
    if launch_pull {
        run::pull_base_branch(&launch, Some(&base), &base);
    }

    if interrupt::requested() {
        return Err(anyhow!("interrupted").into());
    }
    let worktree = ReviewWorktree::create(&launch, &repo.name, &base)?;
    let logs = Logs::of_architect_run(&repo, logs_dir, &timestamp);
    let mut log = logs.path(REVIEW);
    review(worktree, &base, focus, &logs, &mut log)
        .and_then(|final_message| conclude(final_message.as_deref(), &origin, started))
        .map_err(|error| FailedRun {
            log: log.exists().then_some(log),
            ..FailedRun::from(error)
        })
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

/// End the Architect run on the issue the last line of the review's
/// `final_message` names: a plan is marked ready, and an idea issue, or the
/// issue that already covers the top recommendation, is left as it is. Fails if the session had no
/// final message, if its last line is not one the prompt asks for, or if the
/// plan can't be marked ready.
fn conclude(final_message: Option<&str>, origin: &str, started: DateTime<Utc>) -> Result<Outcome> {
    match final_message.and_then(Report::read) {
        Some(Report::Plan(plan)) => {
            mark_plan_ready(&plan, origin, started)?;
            Ok(Outcome::PlanReady(plan))
        }
        Some(Report::Idea(idea)) => Ok(Outcome::IdeaFiled(idea)),
        Some(Report::AlreadyFiled(issue)) => Ok(Outcome::AlreadyFiled(issue)),
        None => bail!("the Architecture review ended without the final line its prompt asks for"),
    }
}

/// Mark `plan` ready: check it, then swap its `needs-triage` for
/// `ready-for-agent`, in one request. Fails, changing no label, if the plan
/// fails its checks.
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
        "marking the plan ready: swapping {NEEDS_TRIAGE} for {READY_FOR_AGENT} on #{}",
        plan.number
    ));
    let mut labels: Vec<&str> = kept
        .iter()
        .map(String::as_str)
        .filter(|&label| label != READY_FOR_AGENT)
        .collect();
    labels.push(READY_FOR_AGENT);
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
