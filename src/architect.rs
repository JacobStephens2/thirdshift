//! An Architect run: an Architecture review of the Base branch, the one its
//! command named or else the one checked out in the Launch directory, from
//! the Launch directory with no Issue URL, then the checks on the Architect
//! plan it published and the label change that marks it ready and labels it
//! `architect-plan`, then, unless the command asked to stop at the plan, the
//! plan dispatched on the Base branch. A review that found no Strong
//! candidate published no plan, and the Architect run ends on the issue it
//! named instead, labelled an
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

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};

use crate::asks::Flags;
use crate::config::UserConfig;
use crate::failed_run::FailedRun;
use crate::github::ListedIssue;
use crate::harness::Choice;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label, Labels, NEEDS_TRIAGE, READY_FOR_AGENT};
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::{Dispatch, LaunchAndGitHub, Outside};
use crate::prompt;
use crate::ready::ReadyIssue;
use crate::run::Ended;

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

/// How an Architect run ended.
pub enum Outcome {
    /// Skipped, before any review, having done nothing.
    Skipped(Skipped),
    /// It was not skipped: how its Architecture review ended, failing if
    /// anything before it did, and how the run it dispatched its plan as
    /// ended, if it dispatched one.
    Ran {
        review: Result<Reviewed, FailedRun>,
        dispatched: Option<Ended>,
    },
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
    /// The plan the review published is marked ready.
    PlanReady(IssueUrl),
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
            Self::PlanReady(_) => "plan published",
            Self::IdeaFiled(_) => "idea filed",
            Self::AlreadyFiled(_) => "idea already filed",
        }
    }

    /// The URL of the issue the Architecture review ended on.
    pub fn url(&self) -> &str {
        match self {
            Self::PlanReady(issue) | Self::IdeaFiled(issue) | Self::AlreadyFiled(issue) => {
                &issue.url
            }
        }
    }
}

impl fmt::Display for Reviewed {
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

/// Run an Architecture review of the Base branch, pointed at `focus` if
/// given, and mark the plan it publishes ready, then, unless `plan_only`,
/// dispatch it as `thirdshift <plan URL>` with `flags` would run it, but on
/// the Base branch. The Base branch is `base`, the branch the command named,
/// whatever the Launch directory has checked out, or without one the branch
/// checked out there. A review that reports an idea issue it filed, or the
/// open issue that already covers its top recommendation, instead of a plan,
/// has that issue labelled an Architect idea, and dispatches nothing. With
/// `config`'s `launch.pull`, the Launch directory's checkout of the Base
/// branch, if that is the branch checked out, is first brought up to date
/// with origin. The review's worktree is gone when the review is concluded.
/// A failure after the plan is published leaves its labels as the review
/// left them. A failure before the review, the preflight checks included, is
/// a failed review too.
///
/// Once the preflight checks pass, and before anything else, the Architect
/// run is skipped, with nothing done, if another on the same repository, or a
/// Pickup run, is still running on this machine: see [`launch::start`]. A
/// skip is recorded in the repository's Activity log. The rest, from the
/// gates to the dispatch, is [`run_through`], through the Launch directory,
/// GitHub, the logs and `harness`, the Harness, Model and Effort its review,
/// and the run it dispatches, run their sessions on, and `flags` and
/// `config`, which ask that run.
pub fn run(
    focus: Option<&str>,
    base: Option<&str>,
    plan_only: bool,
    flags: &Flags,
    config: &UserConfig,
    harness: &mut Choice,
) -> Outcome {
    let started = Utc::now();
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base) {
        Ok(Start::Clear(launch)) => launch,
        Ok(Start::AlreadyRunning(running)) => {
            logs::skipped(Pass::ArchitectRun, &running.0, &running);
            return Outcome::Skipped(Skipped::AlreadyRunning(running));
        }
        Err(error) => return failed(error.into()),
    };
    let mut outside = LaunchAndGitHub {
        launch: &directory,
        repo: &repo,
        harness,
        flags,
        config,
    };
    let architect_run = ArchitectRun {
        repo: &repo,
        base: &base,
        origin: directory.origin(),
        focus,
        started,
        pull: config.launch_pull,
        plan_only,
    };
    run_through(&mut outside, &architect_run)
}

/// An Architect run past its lock and preflight checks: what it runs on,
/// and what it was asked for.
struct ArchitectRun<'a> {
    /// The repository it runs on.
    repo: &'a Repo,
    /// The Base branch.
    base: &'a str,
    /// The URL of the Launch directory's `origin`, which its plan must be in.
    origin: &'a str,
    /// What the command pointed the review at, if anything.
    focus: Option<&'a str>,
    /// When the Architect run started, before its plan can have been
    /// created.
    started: DateTime<Utc>,
    /// Whether to pull the Launch directory's checkout of the Base branch
    /// first.
    pull: bool,
    /// Whether to stop at the plan, dispatching nothing.
    plan_only: bool,
}

/// [`run`], once it holds its repository's lock, through `outside`: its
/// [`gates`], a skip recorded with its reason if one skips; then the check of
/// its Harness, failing before any work if it can't run; then the record
/// that it started work, which keeps its Command log with its repository's
/// logs, where its Session logs go too; then the Launch directory's pull, if
/// asked for. Then, unless it was interrupted, the Architecture review
/// session and its [`conclude`]; then, unless `plan_only`, the plan it
/// marked ready dispatched on its Base branch.
fn run_through(outside: &mut impl Outside, architect_run: &ArchitectRun) -> Outcome {
    match gates(outside) {
        Ok(Decision::GoAhead) => {}
        Ok(Decision::Skip(skipped)) => {
            outside.skipped(Pass::ArchitectRun, &skipped);
            return Outcome::Skipped(skipped);
        }
        Err(error) => return failed(error.into()),
    }
    let review = up_to_the_dispatch(outside, architect_run);
    let dispatched = match &review {
        Ok(Reviewed::PlanReady(plan)) if !architect_run.plan_only => {
            outside.step(format!(
                "dispatching the plan {url}, as thirdshift {url} would",
                url = plan.url
            ));
            Some(outside.dispatch(Dispatch::ArchitectPlan {
                plan,
                base: architect_run.base,
            }))
        }
        _ => None,
    };
    Outcome::Ran { review, dispatched }
}

/// The Architect run failed, as `failed` says, before any review got
/// anywhere: nothing to dispatch.
fn failed(failed: FailedRun) -> Outcome {
    Outcome::Ran {
        review: Err(failed),
        dispatched: None,
    }
}

/// [`run_through`] past its gates, up to its dispatch: the Harness check,
/// the start, the pull, and the Architecture review, with its conclusion,
/// failing with the review's Session log if it gets that far.
fn up_to_the_dispatch(
    outside: &mut impl Outside,
    architect_run: &ArchitectRun,
) -> Result<Reviewed, FailedRun> {
    let ArchitectRun {
        repo,
        base,
        origin,
        focus,
        started,
        pull,
        ..
    } = *architect_run;
    outside.check_harness()?;
    outside.started(Work::ArchitectRun(repo));
    if pull {
        outside.pull(base);
    }
    if outside.interrupted() {
        return Err(FailedRun {
            interrupted: true,
            ..FailedRun::from(anyhow!("interrupted"))
        });
    }
    let starting = match focus {
        Some(focus) => format!("starting the Architecture review of {base}, focused on: {focus}"),
        None => format!("starting the Architecture review of {base}"),
    };
    let prompt = prompt::architecture_review(base, focus);
    let (reviewed, log) = outside.review(base, starting, &prompt, |outside, final_message| {
        conclude(outside, final_message, origin, started)
    });
    reviewed.map_err(|error| FailedRun {
        log,
        ..FailedRun::from(error)
    })
}

/// What an Architect run's gates decided.
enum Decision {
    /// It is skipped, for this reason.
    Skip(Skipped),
    /// It goes ahead to its review.
    GoAhead,
}

/// The gates of an Architect run, reaching its repository through `outside`: a
/// skip for the open Architect plans, if it has any, then for the
/// Architect ideas still labelled `needs-triage`, then for its Ready issue,
/// by the Pickup run's own search. No gate is read once one skips.
fn gates(outside: &mut impl Outside) -> Result<Decision> {
    let open_plans = outside.open_issues(ARCHITECT_PLAN)?;
    if !open_plans.is_empty() {
        return Ok(Decision::Skip(Skipped::OpenPlans(open_plans)));
    }
    let waiting: Vec<_> = outside
        .open_issues(ARCHITECT_IDEA)?
        .into_iter()
        .filter(|idea| idea.labels.has(NEEDS_TRIAGE))
        .collect();
    if !waiting.is_empty() {
        return Ok(Decision::Skip(Skipped::IdeasWaiting(waiting)));
    }
    Ok(match outside.ready_issue()? {
        Some(ReadyIssue { listed, .. }) => Decision::Skip(Skipped::ReadyIssue(listed)),
        None => Decision::GoAhead,
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

/// End the Architecture review on the issue the last line of its
/// `final_message` names, reaching it through `outside`: a
/// plan is marked ready, and an idea issue, or the issue that already covers
/// the top recommendation, is labelled an Architect idea. Fails, with no
/// call made, if the session had no final message or its last line is not
/// one the prompt asks for, and fails if the plan can't be marked ready or
/// the idea labelled.
fn conclude(
    outside: &mut impl Outside,
    final_message: Option<&str>,
    origin: &str,
    started: DateTime<Utc>,
) -> Result<Reviewed> {
    match final_message.and_then(Report::read) {
        Some(Report::Plan(plan)) => {
            mark_plan_ready(outside, &plan, origin, started)?;
            Ok(Reviewed::PlanReady(plan))
        }
        Some(Report::Idea(idea)) => {
            label_idea(outside, &idea)?;
            Ok(Reviewed::IdeaFiled(idea))
        }
        Some(Report::AlreadyFiled(issue)) => {
            label_idea(outside, &issue)?;
            Ok(Reviewed::AlreadyFiled(issue))
        }
        None => bail!("the Architecture review ended without the final line its prompt asks for"),
    }
}

/// Mark `plan` ready: check it, then swap its `needs-triage` for
/// `ready-for-agent` and label it `architect-plan`, in one request that
/// keeps its other labels, as viewed for the checks, having added each label
/// it puts on to the repository if it lacks it. Fails, changing no label, if
/// the plan fails its checks.
fn mark_plan_ready(
    outside: &mut impl Outside,
    plan: &IssueUrl,
    origin: &str,
    started: DateTime<Utc>,
) -> Result<()> {
    outside.step(format!(
        "the Architecture review published the plan {}",
        plan.url
    ));
    let labels = check_plan(outside, plan, origin, started)?;
    if outside.interrupted() {
        bail!("interrupted");
    }
    outside.step(format!(
        "marking the plan ready: swapping {NEEDS_TRIAGE} for {READY_FOR_AGENT} and adding {ARCHITECT_PLAN} on #{}",
        plan.number
    ));
    outside.apply(&Edit::of(
        plan,
        labels,
        &[NEEDS_TRIAGE],
        &[READY_FOR_AGENT, ARCHITECT_PLAN],
    ))
}

/// Label `idea` an Architect idea: put `needs-triage` and `architect-idea`
/// on it, in one request that keeps its other labels, as viewed, having
/// added each to the repository if it lacks it. `needs-triage` goes back on
/// an issue that had been triaged: the factory again takes it for the best
/// next move.
fn label_idea(outside: &mut impl Outside, idea: &IssueUrl) -> Result<()> {
    if outside.interrupted() {
        bail!("interrupted");
    }
    outside.step(format!(
        "labelling #{} an Architect idea: adding {NEEDS_TRIAGE} and {ARCHITECT_IDEA}",
        idea.number
    ));
    outside
        .issue(idea)
        .and_then(|viewed| {
            outside.apply(&Edit::of(
                idea,
                viewed.labels,
                &[],
                &[NEEDS_TRIAGE, ARCHITECT_IDEA],
            ))
        })
        .with_context(|| format!("could not label the Architect idea #{}", idea.number))
}

/// Check `plan`, viewing it: fails unless it is in the repository at
/// `origin`, which is checked before the view, open, created since the
/// Architect run `started`, and has no label but `needs-triage` that makes
/// an Unready Ticket. Returns its labels, as viewed.
fn check_plan(
    outside: &mut impl Outside,
    plan: &IssueUrl,
    origin: &str,
    started: DateTime<Utc>,
) -> Result<Labels> {
    if !plan.matches_origin(origin) {
        bail!(
            "the plan {} is not in the repository at origin {origin}",
            plan.url
        );
    }
    let issue = outside.issue(plan)?;
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
    if let Some(unready) = issue.labels.swapped(&[NEEDS_TRIAGE], &[]).unready() {
        bail!("the plan {} is labelled {unready}", plan.url);
    }
    Ok(issue.labels)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::github;
    use crate::pass::{Call, InMemory};
    use crate::run::{Goal, Reached};

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

    /// The gates of an Architect run on `repo`: the reason it is skipped,
    /// as its line, or `None` for the go-ahead, the URLs the skip names,
    /// and what the gates did.
    fn decide(mut repo: InMemory) -> (Option<String>, Vec<String>, Vec<Call>) {
        let decision = gates(&mut repo).unwrap();
        let (line, urls) = match decision {
            Decision::Skip(skipped) => (
                Some(skipped.to_string()),
                skipped.urls().into_iter().map(String::from).collect(),
            ),
            Decision::GoAhead => (None, Vec::new()),
        };
        (line, urls, repo.calls)
    }

    /// The URL of issue `number` in `acme/widgets`.
    fn url(number: u64) -> String {
        format!("https://github.com/acme/widgets/issues/{number}")
    }

    const PLANS: Call = Call::Open("architect-plan");
    const IDEAS: Call = Call::Open("architect-idea");

    #[test]
    fn with_no_plan_idea_or_ready_issue_the_architect_run_goes_ahead_having_read_each_gate() {
        // #7 is a plan a review that failed left half-published, never
        // labelled an Architect plan nor an Architect idea.
        let repo = InMemory::default()
            .issue(3, false, &["architect-plan"])
            .issue(4, false, &["architect-idea", "needs-triage"])
            .issue(7, true, &["needs-triage", "architecture"]);

        let (line, _, calls) = decide(repo);

        assert_eq!(line, None);
        assert_eq!(calls, [PLANS, IDEAS, Call::ReadySearch]);
    }

    #[test]
    fn an_open_plan_skips_naming_each_plan_with_no_read_after() {
        let repo = InMemory::default()
            .issue(5, true, &["architect-plan"])
            .issue(6, true, &["architect-plan", "ready-for-agent"])
            .issue(7, true, &["architect-idea", "needs-triage"])
            .ready(8, false);

        let (line, urls, calls) = decide(repo);

        assert_eq!(
            line.unwrap(),
            format!(
                "Architect plan #5 \"Issue 5\" is still open: pick it up with thirdshift {}; \
                 Architect plan #6 \"Issue 6\" is still open: pick it up with thirdshift {}",
                url(5),
                url(6)
            )
        );
        assert_eq!(urls, [url(5), url(6)]);
        assert_eq!(calls, [PLANS]);
    }

    #[test]
    fn a_waiting_idea_skips_naming_each_waiting_idea_with_no_ready_issue_search() {
        let repo = InMemory::default()
            .issue(5, true, &["architect-idea", "needs-triage"])
            .issue(6, true, &["architect-idea", "ready-for-agent"])
            .issue(7, true, &["Architect-Idea", "Needs-Triage"])
            .ready(8, false);

        let (line, urls, calls) = decide(repo);

        assert_eq!(
            line.unwrap(),
            format!(
                "Architect idea #5 \"Issue 5\" is waiting for triage: {}; \
                 Architect idea #7 \"Issue 7\" is waiting for triage: {}",
                url(5),
                url(7)
            )
        );
        assert_eq!(urls, [url(5), url(7)]);
        assert_eq!(calls, [PLANS, IDEAS]);
    }

    #[test]
    fn an_idea_no_longer_labelled_needs_triage_does_not_skip() {
        for decided in ["ready-for-agent", "wontfix", "ready-for-human"] {
            let repo = InMemory::default().issue(5, true, &["architect-idea", decided]);

            let (line, _, calls) = decide(repo);

            assert_eq!(line, None, "{decided}");
            assert_eq!(calls, [PLANS, IDEAS, Call::ReadySearch], "{decided}");
        }
    }

    #[test]
    fn a_ready_issue_skips_naming_it() {
        let repo = InMemory::default().ready(9, true);

        let (line, urls, calls) = decide(repo);

        assert_eq!(
            line.unwrap(),
            format!("Ready issue #9 \"Issue 9\" goes first: {}", url(9))
        );
        assert_eq!(urls, [url(9)]);
        assert_eq!(calls, [PLANS, IDEAS, Call::ReadySearch]);
    }

    #[test]
    fn an_open_plan_wins_over_a_waiting_idea_and_a_ready_issue() {
        for idea in [true, false] {
            let mut repo = InMemory::default()
                .issue(5, true, &["architect-plan"])
                .ready(8, false);
            if idea {
                repo = repo.issue(6, true, &["architect-idea", "needs-triage"]);
            }

            let (_, urls, _) = decide(repo);

            assert_eq!(urls, [url(5)], "with an idea waiting: {idea}");
        }
    }

    #[test]
    fn an_idea_triaged_ready_for_agent_skips_as_the_ready_issue_it_is() {
        let repo = InMemory::default()
            .issue(7, true, &["architect-idea", "ready-for-agent"])
            .ready(7, false);

        let (line, urls, calls) = decide(repo);

        assert!(line.unwrap().starts_with("Ready issue #7"));
        assert_eq!(urls, [url(7)]);
        assert_eq!(calls, [PLANS, IDEAS, Call::ReadySearch]);
    }

    #[test]
    fn a_waiting_idea_wins_over_a_ready_issue() {
        let repo = InMemory::default()
            .issue(6, true, &["architect-idea", "needs-triage"])
            .ready(8, false);

        let (line, urls, _) = decide(repo);

        assert!(line.unwrap().starts_with("Architect idea #6"));
        assert_eq!(urls, [url(6)]);
    }

    const ORIGIN: &str = "git@github.com:acme/widgets.git";

    /// When the Architect run started.
    fn started() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-04T09:00:00.750Z")
            .unwrap()
            .to_utc()
    }

    /// An issue as viewed: open or not, created `after` seconds after the
    /// Architect run started, labelled `labels`.
    fn viewed(open: bool, after: i64, labels: &[&str]) -> github::Issue {
        github::Issue {
            is_open: open,
            labels: labels.iter().copied().collect(),
            created: started() + chrono::Duration::seconds(after),
        }
    }

    /// The final message of a review whose last line is `line` and the URL
    /// of issue 8.
    fn message(line: &str) -> String {
        format!("Reviewed the codebase.\n\n{line}{}\n", url(8))
    }

    /// The Architect run's conclusion on `final_message`, through `repo`:
    /// how the review ended, or the cause it failed with, and what it did.
    fn concluded(mut repo: InMemory, final_message: &str) -> (Result<String, String>, Vec<Call>) {
        let ended = conclude(&mut repo, Some(final_message), ORIGIN, started())
            .map(|reviewed| reviewed.to_string())
            .map_err(|error| format!("{error:#}"));
        (ended, repo.calls)
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    fn edit(issue: u64, off: &[&'static str], on: &[&'static str], labels: &[&str]) -> Call {
        Call::Edit {
            issue,
            off: off.to_vec(),
            on: on.to_vec(),
            labels: labels.iter().map(|label| label.to_string()).collect(),
        }
    }

    const PUBLISHED: &str =
        "the Architecture review published the plan https://github.com/acme/widgets/issues/8";
    const MARKING: &str = "marking the plan ready: swapping needs-triage for ready-for-agent and adding architect-plan on #8";
    const LABELLING: &str =
        "labelling #8 an Architect idea: adding needs-triage and architect-idea";

    #[test]
    fn a_plan_is_viewed_once_and_marked_ready_in_one_edit_keeping_its_other_labels() {
        let repo = InMemory::default().viewable(
            8,
            viewed(true, 0, &["architecture", "needs-triage", "Spec"]),
        );

        let (ended, calls) = concluded(repo, &message(prompt::PLAN_LINE));

        assert_eq!(
            ended.unwrap(),
            format!("plan {} is ready for an agent", url(8))
        );
        assert_eq!(
            calls,
            [
                step(PUBLISHED),
                Call::View(8),
                step(MARKING),
                edit(
                    8,
                    &["needs-triage"],
                    &["ready-for-agent", "architect-plan"],
                    &["architecture", "Spec", "ready-for-agent", "architect-plan"],
                ),
            ]
        );
    }

    #[test]
    fn a_plan_with_needs_triage_in_another_case_has_it_taken_off() {
        let repo = InMemory::default().viewable(8, viewed(true, 0, &["Needs-Triage"]));

        let (ended, calls) = concluded(repo, &message(prompt::PLAN_LINE));

        assert!(ended.is_ok());
        assert_eq!(
            calls.last().unwrap(),
            &edit(
                8,
                &["needs-triage"],
                &["ready-for-agent", "architect-plan"],
                &["ready-for-agent", "architect-plan"],
            )
        );
    }

    #[test]
    fn a_plan_in_another_repository_is_refused_without_a_view() {
        let repo = InMemory::default().viewable(8, viewed(true, 0, &["needs-triage"]));
        let other = "Architecture review plan: https://github.com/other/widgets/issues/8";

        let (ended, calls) = concluded(repo, other);

        assert_eq!(
            ended.unwrap_err(),
            format!(
                "the plan https://github.com/other/widgets/issues/8 is not in the repository at origin {ORIGIN}"
            )
        );
        assert_eq!(
            calls,
            [step(
                "the Architecture review published the plan https://github.com/other/widgets/issues/8"
            )]
        );
    }

    #[test]
    fn a_plan_failing_a_check_is_refused_with_its_cause_and_no_edit() {
        for (plan, cause) in [
            (viewed(false, 0, &["needs-triage"]), "is closed"),
            (
                viewed(true, -1, &["needs-triage"]),
                "was created before this Architect run started",
            ),
            (
                viewed(true, 0, &["needs-triage", "ready-for-human"]),
                "is labelled ready-for-human",
            ),
            (
                viewed(true, 0, &["needs-triage", "Needs-Info"]),
                "is labelled needs-info",
            ),
            (viewed(true, 0, &["wontfix"]), "is labelled wontfix"),
        ] {
            let repo = InMemory::default().viewable(8, plan);

            let (ended, calls) = concluded(repo, &message(prompt::PLAN_LINE));

            assert_eq!(ended.unwrap_err(), format!("the plan {} {cause}", url(8)));
            assert_eq!(calls, [step(PUBLISHED), Call::View(8)], "{cause}");
        }
    }

    #[test]
    fn a_plan_created_in_the_second_the_run_started_passes() {
        // The run started 0.75 s into its second; GitHub gives the plan's
        // creation to the second.
        let mut plan = viewed(true, 0, &["needs-triage"]);
        plan.created = DateTime::parse_from_rfc3339("2026-10-04T09:00:00Z")
            .unwrap()
            .to_utc();
        let repo = InMemory::default().viewable(8, plan);

        let (ended, _) = concluded(repo, &message(prompt::PLAN_LINE));

        assert!(ended.is_ok(), "{ended:?}");
    }

    #[test]
    fn an_interrupt_after_the_plans_checks_makes_no_edit() {
        let repo = InMemory::default()
            .viewable(8, viewed(true, 0, &["needs-triage"]))
            .interrupted();

        let (ended, calls) = concluded(repo, &message(prompt::PLAN_LINE));

        assert_eq!(ended.unwrap_err(), "interrupted");
        assert_eq!(calls, [step(PUBLISHED), Call::View(8)]);
    }

    #[test]
    fn an_idea_filed_or_already_filed_is_labelled_an_architect_idea_keeping_its_other_labels() {
        for (line, ending) in [
            (
                prompt::IDEA_LINE,
                format!(
                    "no Strong candidate: the Architecture review filed the idea {}",
                    url(8)
                ),
            ),
            (
                prompt::ALREADY_FILED_LINE,
                format!(
                    "no Strong candidate: {} already covers the Architecture review's top recommendation, so it filed nothing",
                    url(8)
                ),
            ),
        ] {
            let repo = InMemory::default()
                .viewable(8, viewed(true, -3600, &["architecture", "needs-triage"]));

            let (ended, calls) = concluded(repo, &message(line));

            assert_eq!(ended.unwrap(), ending);
            assert_eq!(
                calls,
                [
                    step(LABELLING),
                    Call::View(8),
                    edit(
                        8,
                        &[],
                        &["architect-idea"],
                        &["architecture", "needs-triage", "architect-idea"],
                    ),
                ],
                "{line}"
            );
        }
    }

    #[test]
    fn an_idea_already_filed_and_triaged_goes_back_to_needs_triage_keeping_its_other_labels() {
        for (triaged, on, after) in [
            (
                &["ready-for-human", "architecture"][..],
                &["needs-triage", "architect-idea"][..],
                &[
                    "ready-for-human",
                    "architecture",
                    "needs-triage",
                    "architect-idea",
                ][..],
            ),
            (
                &["Architecture", "WONTFIX", "Architect-Idea"],
                &["needs-triage"],
                &["Architecture", "WONTFIX", "needs-triage", "architect-idea"],
            ),
            (
                &["needs-info"],
                &["needs-triage", "architect-idea"],
                &["needs-info", "needs-triage", "architect-idea"],
            ),
            (
                &[],
                &["needs-triage", "architect-idea"],
                &["needs-triage", "architect-idea"],
            ),
        ] {
            let repo = InMemory::default().viewable(8, viewed(true, -3600, triaged));

            let (ended, calls) = concluded(repo, &message(prompt::ALREADY_FILED_LINE));

            assert!(ended.is_ok(), "{triaged:?}: {ended:?}");
            assert_eq!(
                calls,
                [step(LABELLING), Call::View(8), edit(8, &[], on, after)],
                "{triaged:?}"
            );
        }
    }

    #[test]
    fn an_idea_that_cannot_be_viewed_fails_naming_it_ahead_of_the_cause() {
        let (ended, calls) = concluded(InMemory::default(), &message(prompt::IDEA_LINE));

        assert_eq!(
            ended.unwrap_err(),
            "could not label the Architect idea #8: gh: Could not resolve to an issue with the number of 8"
        );
        assert_eq!(calls, [step(LABELLING), Call::View(8)]);
    }

    #[test]
    fn an_idea_that_cannot_be_edited_fails_naming_it_ahead_of_the_cause() {
        let repo = InMemory::default()
            .viewable(8, viewed(true, 0, &["bug"]))
            .edit_failing(8);

        let (ended, calls) = concluded(repo, &message(prompt::ALREADY_FILED_LINE));

        assert_eq!(
            ended.unwrap_err(),
            "could not label the Architect idea #8: gh: could not edit #8"
        );
        assert_eq!(
            calls,
            [
                step(LABELLING),
                Call::View(8),
                edit(
                    8,
                    &[],
                    &["needs-triage", "architect-idea"],
                    &["bug", "needs-triage", "architect-idea"],
                ),
            ]
        );
    }

    #[test]
    fn an_interrupt_before_an_idea_is_viewed_makes_no_call() {
        let repo = InMemory::default()
            .viewable(8, viewed(true, 0, &[]))
            .interrupted();

        let (ended, calls) = concluded(repo, &message(prompt::IDEA_LINE));

        assert_eq!(ended.unwrap_err(), "interrupted");
        assert_eq!(calls, []);
    }

    #[test]
    fn a_final_message_without_the_line_as_asked_for_fails_with_no_call_made() {
        let repo = InMemory::default().viewable(8, viewed(true, 0, &["needs-triage"]));

        let (ended, calls) = concluded(repo, "Published the plan.");

        assert_eq!(
            ended.unwrap_err(),
            "the Architecture review ended without the final line its prompt asks for"
        );
        assert_eq!(calls, []);
    }

    #[test]
    fn no_final_message_fails_with_no_call_made() {
        let mut repo = InMemory::default();

        let ended = conclude(&mut repo, None, ORIGIN, started());

        assert_eq!(
            ended.unwrap_err().to_string(),
            "the Architecture review ended without the final line its prompt asks for"
        );
        assert_eq!(repo.calls, []);
    }

    /// The repository every Architect run is on.
    fn widgets() -> Repo {
        Repo {
            owner: "acme".to_string(),
            name: "widgets".to_string(),
        }
    }

    /// An Architect run on `repo`, past its lock, with no focus, no pull,
    /// and no plan-only, on the Base branch `main`, except as `asked`
    /// changes them, its dispatched run ending ready for review: how it
    /// ended, and what it did outside itself.
    fn architect(
        repo: InMemory,
        asked: impl FnOnce(&mut ArchitectRun<'_>),
    ) -> (Outcome, Vec<Call>) {
        let mut repo = repo.dispatched_ending(ready_for_review());
        let widgets = widgets();
        let mut architect_run = ArchitectRun {
            repo: &widgets,
            base: "main",
            origin: ORIGIN,
            focus: None,
            started: started(),
            pull: false,
            plan_only: false,
        };
        asked(&mut architect_run);
        let outcome = run_through(&mut repo, &architect_run);
        (outcome, repo.calls)
    }

    /// A dispatched run that ended ready for review.
    fn ready_for_review() -> Ended {
        Ended {
            outcome: Ok(Reached {
                pr_url: "https://github.com/acme/widgets/pull/12".to_string(),
                goal: Goal::ReadyForReview,
                log: None,
                ticket_lines: Vec::new(),
            }),
            base_fix: None,
            advice: Vec::new(),
        }
    }

    /// What an Architect run that got past its gates came to: how its review
    /// ended, as its line or its cause, and whether it dispatched a run.
    fn reviewed(outcome: Outcome) -> (Result<String, String>, bool) {
        let Outcome::Ran { review, dispatched } = outcome else {
            panic!("the Architect run was skipped");
        };
        let review = review
            .map(|reviewed| reviewed.to_string())
            .map_err(|failed| format!("{:#}", failed.error));
        (review, dispatched.is_some())
    }

    /// The calls that start the Architecture review of `main`, with no
    /// focus.
    fn reviewing_main() -> [Call; 2] {
        [
            Call::Review {
                base: "main".to_string(),
                prompt: prompt::architecture_review("main", None),
            },
            step("starting the Architecture review of main"),
        ]
    }

    /// A repository whose review publishes plan #8, which passes its checks.
    fn publishing_a_plan() -> InMemory {
        InMemory::default()
            .reviewed(&message(prompt::PLAN_LINE))
            .viewable(8, viewed(true, 0, &["needs-triage"]))
    }

    #[test]
    fn a_skipped_architect_run_records_its_reason_with_no_harness_check_start_pull_or_review() {
        for (repo, gates_read) in [
            (InMemory::default().issue(5, true, &["architect-plan"]), 1),
            (
                InMemory::default().issue(5, true, &["architect-idea", "needs-triage"]),
                2,
            ),
            (InMemory::default().ready(5, false), 3),
        ] {
            let repo = repo.reviewed(&message(prompt::PLAN_LINE));

            let (outcome, calls) = architect(repo, |run| run.pull = true);

            let Outcome::Skipped(skipped) = outcome else {
                panic!("the Architect run was not skipped");
            };
            let mut expected: Vec<Call> = [PLANS, IDEAS, Call::ReadySearch]
                .into_iter()
                .take(gates_read)
                .collect();
            expected.push(Call::Skipped(skipped.to_string()));
            assert_eq!(calls, expected, "{skipped}");
        }
    }

    #[test]
    fn a_harness_that_fails_its_check_fails_the_architect_run_before_its_start() {
        let repo = publishing_a_plan().harness_failing();

        let (outcome, calls) = architect(repo, |run| run.pull = true);

        assert_eq!(
            reviewed(outcome),
            (Err("claude is not on PATH".to_string()), false)
        );
        assert_eq!(calls, [PLANS, IDEAS, Call::ReadySearch, Call::HarnessCheck]);
    }

    #[test]
    fn the_pull_is_made_only_when_asked_after_the_start_is_recorded() {
        for pull in [false, true] {
            let (_, calls) = architect(publishing_a_plan(), |run| run.pull = pull);

            let mut expected = vec![
                PLANS,
                IDEAS,
                Call::ReadySearch,
                Call::HarnessCheck,
                Call::Started(None),
            ];
            if pull {
                expected.push(Call::Pull("main".to_string()));
            }
            expected.extend(reviewing_main());
            assert_eq!(calls[..expected.len()], expected, "pull: {pull}");
        }
    }

    #[test]
    fn an_interrupt_after_the_gates_fails_the_architect_run_as_interrupted_with_no_review() {
        let repo = publishing_a_plan().interrupted();

        let (outcome, calls) = architect(repo, |run| run.pull = true);

        let Outcome::Ran {
            review: Err(failed),
            dispatched: None,
        } = outcome
        else {
            panic!("the Architect run did not fail, or dispatched a run");
        };
        assert_eq!(failed.error.to_string(), "interrupted");
        assert!(failed.interrupted);
        assert_eq!(failed.log, None);
        assert_eq!(
            calls,
            [
                PLANS,
                IDEAS,
                Call::ReadySearch,
                Call::HarnessCheck,
                Call::Started(None),
                Call::Pull("main".to_string()),
            ]
        );
    }

    #[test]
    fn the_reviews_prompt_carries_the_base_branch_and_the_focus() {
        let (_, calls) = architect(publishing_a_plan(), |run| {
            run.base = "develop";
            run.focus = Some("the Spec run");
        });

        let review = calls
            .iter()
            .position(|call| matches!(call, Call::Review { .. }))
            .expect("no review session");
        let Call::Review { base, prompt } = &calls[review] else {
            unreachable!();
        };
        assert_eq!(
            calls[review + 1],
            step("starting the Architecture review of develop, focused on: the Spec run")
        );
        assert_eq!(base, "develop");
        assert!(
            prompt.contains("the base branch develop:"),
            "prompt: {prompt}"
        );
        assert!(
            prompt.contains("Focus the review on: the Spec run\n"),
            "prompt: {prompt}"
        );
    }

    #[test]
    fn a_plan_is_dispatched_on_the_base_branch_after_the_label_swap_unless_plan_only() {
        for plan_only in [false, true] {
            let (outcome, calls) = architect(publishing_a_plan(), |run| {
                run.base = "develop";
                run.plan_only = plan_only;
            });

            assert_eq!(
                reviewed(outcome),
                (
                    Ok(format!("plan {} is ready for an agent", url(8))),
                    !plan_only
                )
            );
            let swap = calls
                .iter()
                .position(|call| matches!(call, Call::Edit { .. }))
                .expect("the plan was not marked ready");
            let dispatch = [
                step(&format!(
                    "dispatching the plan {url}, as thirdshift {url} would",
                    url = url(8)
                )),
                Call::DispatchPlan {
                    plan: 8,
                    base: "develop".to_string(),
                },
            ];
            let after_swap: &[Call] = if plan_only { &[] } else { &dispatch };
            assert_eq!(calls[swap + 1..], *after_swap, "plan-only: {plan_only}");
        }
    }

    #[test]
    fn an_idea_filed_or_already_filed_dispatches_nothing() {
        for line in [prompt::IDEA_LINE, prompt::ALREADY_FILED_LINE] {
            let repo = InMemory::default()
                .reviewed(&message(line))
                .viewable(8, viewed(true, -3600, &[]));

            let (outcome, calls) = architect(repo, |_| {});

            let (review, dispatched) = reviewed(outcome);
            assert!(review.is_ok(), "{line}: {review:?}");
            assert!(!dispatched, "{line}");
            assert!(
                matches!(calls.last(), Some(Call::Edit { .. })),
                "{line}: {calls:?}"
            );
        }
    }

    const SESSION_LOG: &str = "/logs/acme/widgets/sessions/architect-1-architecture-review.jsonl";

    #[test]
    fn a_failed_review_dispatches_nothing_and_carries_the_reviews_session_log() {
        for repo in [
            InMemory::default().review_failing("claude exited with status 1"),
            InMemory::default().reviewed("Published the plan."),
            InMemory::default()
                .reviewed(&message(prompt::PLAN_LINE))
                .viewable(8, viewed(false, 0, &["needs-triage"])),
        ] {
            let (outcome, calls) = architect(repo.session_log(SESSION_LOG), |_| {});

            let Outcome::Ran {
                review: Err(failed),
                dispatched: None,
            } = outcome
            else {
                panic!("the review did not fail, or a run was dispatched");
            };
            assert_eq!(
                failed.log.as_deref(),
                Some(Path::new(SESSION_LOG)),
                "{:#}",
                failed.error
            );
            assert!(
                !calls
                    .iter()
                    .any(|call| matches!(call, Call::DispatchPlan { .. })),
                "{:#}",
                failed.error
            );
        }
    }
}
