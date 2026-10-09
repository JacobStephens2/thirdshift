//! One Run: from an Issue URL to a checked PR, or to a Failed run. An issue
//! with sub-issues is a Spec instead, handed to a Spec run.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Result, anyhow};

use crate::asks::Asks;
use crate::base_fix::{Advice, BaseFix};
use crate::branch::{self, Selection};
use crate::child_run::Kind;
use crate::claim::{self, Claim};
use crate::delivery::{Delivery, Opening};
use crate::failed_run::FailedRun;
use crate::github::{GitHub, Ticket};
use crate::harness::Choice;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::launch::LaunchDirectory;
use crate::logs::{self, Work};
use crate::progress;
use crate::prompt;
use crate::session::Logs;
use crate::spec_run;
use crate::worktree::Worktree;

/// The implement session's kind, in its progress lines and log name.
const IMPLEMENT: &str = "implement";

/// Where a Run takes its PR: ready for review, or, in a Merge run, merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Goal {
    ReadyForReview,
    Merged,
}

impl Goal {
    /// What became of the PR once the Run reached this goal, as in
    /// "PR <url> is merged" and a Run notification's subject.
    pub fn outcome(self) -> &'static str {
        match self {
            Goal::ReadyForReview => "ready for review",
            Goal::Merged => "merged",
        }
    }
}

/// A Run, or a Spec run, that reached its goal.
pub struct Reached {
    pub pr_url: String,
    /// The goal reached.
    pub goal: Goal,
    /// The most recent session's log, if a session was started.
    pub log: Option<PathBuf>,
    /// In a Spec run, a line on each Ticket it landed; empty in a Run.
    pub ticket_lines: Vec<String>,
}

/// How a Run, or a Spec run, ended.
pub struct Ended {
    pub outcome: Result<Reached, FailedRun>,
    /// What became of the Base fix it started or waited on, if any, as
    /// [`BaseFix::report`] tells it.
    pub base_fix: Option<String>,
    /// What it says after its cause, if Inherited failures failed it with no
    /// Base fix taken, as [`BaseFix::into_advice`] gives it.
    pub advice: Vec<Advice>,
}

/// What started a Run, or a Spec run, which says where its Base branch comes
/// from when it isn't a Continuation's open pull request that says.
#[derive(Clone, Copy)]
pub enum StartedBy<'a> {
    /// `thirdshift <Issue URL>`: the Base branch is the branch checked out
    /// in the Launch directory.
    Command,
    /// A Pass, dispatching an issue: the Base branch is that Pass's, whatever the
    /// Launch directory has checked out.
    Dispatch { base: &'a str },
    /// Another thirdshift, as this child Run, a Ticket's Run in a Spec run or
    /// a Base fix: the Base branch is the one the child Run was given.
    Child(&'a Kind),
}

impl<'a> StartedBy<'a> {
    /// The child Run this is, if another thirdshift started it.
    fn child(self) -> Option<&'a Kind> {
        match self {
            StartedBy::Child(kind) => Some(kind),
            StartedBy::Command | StartedBy::Dispatch { .. } => None,
        }
    }

    /// Whether the Run, or the Spec run, makes the Claim on its issue: a
    /// child Run makes none, so that only the issue the Day shift would look
    /// at carries one.
    fn makes_claim(self) -> bool {
        self.child().is_none()
    }

    /// The Base branch the Run was given, if what started it gave one.
    fn given_base(self) -> Option<&'a str> {
        match self {
            StartedBy::Command => None,
            StartedBy::Dispatch { base } => Some(base),
            StartedBy::Child(kind) => Some(kind.base()),
        }
    }
}

/// [`run`] the Run on `issue` that `started_by` started and that is asked
/// `asks`, to its end, with the Model and Effort in `asks` settled on the
/// Harness's names for them once checked. Its Run notification, if `asks`
/// has it send one, is for what started the Run to send, once it has ended.
pub fn run_to_end(issue: &IssueUrl, asks: &mut Asks, started_by: StartedBy) -> Ended {
    let mut base_fix = BaseFix::new(
        started_by.child(),
        asks.base_fix.clone(),
        asks.harness.clone(),
        asks.security_fix,
        asks.security_review,
    );
    let outcome = run(issue, asks, started_by, &mut base_fix);
    Ended {
        outcome,
        base_fix: base_fix.report(),
        advice: base_fix.into_advice(),
    }
}

/// Take `issue` to a ready PR, or in a Merge run a merged one. Any failure
/// after the worktree exists, including a merge that fails, goes through the
/// Failed run path. The worktree and the local
/// Issue branch are gone when this returns, except
/// that a Failed run whose work did not reach origin keeps the worktree and
/// branch. With `asks.launch_pull`, the Launch directory's checkout of the
/// Base branch, if that is the branch checked out, is first brought up to
/// date with origin.
///
/// The Launch directory is opened first, with its Origin match, the check
/// that `issue` is open and the git identity check. A Run started by its
/// command then checks the Harness, Model and Effort its sessions run on, as
/// `asks` say, settling the Model and Effort in them, and in `base_fix`, on
/// the Harness's names: one started by another thirdshift, or dispatched by
/// a pass, runs on what that command checked. The rest is [`start`], through the
/// Launch directory, GitHub and the logs.
///
/// `base_fix` is the one Base fix the Run, or a Spec run for its Spec PR, may
/// start, or wait on, when its only red checks are Inherited failures: what
/// `asks` ask about one is already in it.
fn run(
    issue: &IssueUrl,
    asks: &mut Asks,
    started_by: StartedBy,
    base_fix: &mut BaseFix,
) -> Result<Reached, FailedRun> {
    let directory = LaunchDirectory::open_for_run(issue)?;
    if let StartedBy::Command = started_by {
        asks.harness.check()?;
        base_fix.runs_on(asks.harness.clone());
    }
    let asks = &*asks;
    let mut outside = LaunchAndGitHub {
        directory: &directory,
        issue,
        goal: asks.goal,
        security_fix: asks.security_fix,
        security_review: asks.security_review
            && !matches!(started_by, StartedBy::Child(Kind::Ticket { .. })),
        base_fix,
        logs: Logs::of_run(issue),
        harness: &asks.harness,
        claim: None,
    };
    start(
        &mut outside,
        issue,
        asks,
        started_by,
        directory.checked_out(),
    )
}

/// [`run`], once the Launch directory is open, with `checked_out` the branch
/// checked out there, through `outside`.
///
/// The Base branch `started_by` gave the Run, if it gave one, stands in for
/// the checked-out branch as the Base branch: the Spec branch or the Base
/// branch of the Run that started a child Run, or the Base branch of the
/// Pass that dispatched this one. A Continuation's
/// open pull request's base beats either. Unless the Run is a child Run, an
/// issue with sub-issues is a Spec, taken on by a Spec run instead, whose
/// Spec branch is picked like an Issue branch, running as many Tickets at
/// once as `asks.tickets_at_once` says. A `parallel` the command asked for,
/// as `asks.parallel_asked` says, on an issue with no sub-issues fails before
/// any work, as does a Spec whose Tickets are all closed with no Spec branch
/// to continue.
///
/// Once those checks pass, and the Base branch is settled and, if asked,
/// pulled, an interrupt stops the Run. Then, before the worktree is created,
/// the Run, or the Spec run, makes the Claim on `issue`, unless it is a child
/// Run. A Claim that can't be made fails it there, before any work. Once the
/// Run, or the Spec run, has ended, the Claim is ended with the goal it
/// reached, or none if it failed: see [`claim::Claim::end`].
fn start<O: Outside>(
    outside: &mut O,
    issue: &IssueUrl,
    asks: &Asks,
    started_by: StartedBy,
    checked_out: Option<&str>,
) -> Result<Reached, FailedRun> {
    let tickets = match started_by.child() {
        Some(_) => Vec::new(),
        None => outside.tickets()?,
    };
    outside.started(match tickets.is_empty() {
        true => Work::Run(issue),
        false => Work::SpecRun(issue),
    });
    if asks.parallel_asked && tickets.is_empty() {
        return Err(anyhow!(
            "parallel is only for a Spec, and #{} has no sub-issues",
            issue.number
        )
        .into());
    }
    let selection = outside.select()?;
    // A Spec implemented some other way: the Spec run leaves it be.
    if matches!(selection, Selection::Fresh { .. })
        && spec_run::all_closed(tickets.iter().map(|ticket| ticket.is_open))
    {
        return Err(
            anyhow!("every Ticket is closed and there is no Spec branch; nothing to do").into(),
        );
    }
    let given = started_by.given_base();
    let named = selection
        .pr_base(given, checked_out, |line| outside.step(line))
        .or(given);
    let base = outside.prepare_base_branch(named, asks.launch_pull)?;

    if outside.interrupted() {
        return Err(FailedRun {
            interrupted: true,
            ..anyhow!("interrupted").into()
        });
    }
    let claims = started_by.makes_claim();
    if claims {
        outside.make_claim()?;
    }
    let outcome = start_in_worktree(
        outside,
        issue,
        tickets,
        &selection,
        &base,
        asks.tickets_at_once,
    );
    if claims {
        outside.end_claim(outcome.as_ref().ok().map(|reached| reached.goal));
    }
    outcome
}

/// [`start`], from the worktree on: create it for the branch `selection`
/// picked, on the Base branch `base`, and take `issue` there, as a Spec run,
/// running up to `parallel` at once, if it has `tickets`.
fn start_in_worktree<O: Outside>(
    outside: &mut O,
    issue: &IssueUrl,
    tickets: Vec<Ticket>,
    selection: &Selection,
    base: &str,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let branch = selection.branch();
    let (checkout, prompt) = match selection {
        Selection::Fresh { .. } => (Checkout::Fresh, prompt::fresh(issue, base, branch)),
        Selection::Continuation { pr, .. } => (
            Checkout::Continuation,
            prompt::continuation(issue, base, branch, pr.as_ref().map(|pr| pr.url.as_str())),
        ),
    };
    let worktree = outside.worktree(checkout, branch, base)?;
    if !tickets.is_empty() {
        return outside.spec_run(tickets, worktree, base, parallel);
    }
    let opening = Opening {
        kind: IMPLEMENT,
        prompt,
        catch_up_from_origin: false,
    };
    outside.deliver(worktree, base, opening)
}

/// How a Run's worktree comes by its Issue branch: started fresh from the
/// Base branch, or continued from origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checkout {
    Fresh,
    Continuation,
}

/// What a Run's start does or reads outside itself, for its issue: the
/// Tickets, the logs, Issue branch selection, the Launch directory's Base
/// branch, the interrupt, the Claim, the worktree, the Spec run, the
/// Delivery and its progress lines.
trait Outside {
    /// The worktree the Run's work is done in.
    type Worktree;

    /// The issue's Tickets, its sub-issues, if it is a Spec.
    fn tickets(&mut self) -> Result<Vec<Ticket>>;
    /// Record `work` as started.
    fn started(&mut self, work: Work);
    /// Select the Issue branch.
    fn select(&mut self) -> Result<Selection>;
    /// Settle the Base branch from `named`, the branch named for the Run, if
    /// any, else the branch checked out in the Launch directory: see
    /// [`LaunchDirectory::prepare_base_branch`], including its optional
    /// checkout update when `pull` is requested.
    fn prepare_base_branch(&mut self, named: Option<&str>, pull: bool) -> Result<String>;
    /// Whether the Run was interrupted.
    fn interrupted(&mut self) -> bool;
    /// Make the Claim on the issue.
    fn make_claim(&mut self) -> Result<()>;
    /// End the Claim made, with `reached`, the goal the Run reached, or none.
    fn end_claim(&mut self, reached: Option<Goal>);
    /// Create the worktree for the Issue branch `branch`, by `checkout`, on
    /// the Base branch `base`.
    fn worktree(&mut self, checkout: Checkout, branch: &str, base: &str) -> Result<Self::Worktree>;
    /// Take the Spec, from its Spec branch in `worktree`, through a Spec run
    /// into the Base branch `base` with `tickets`, `parallel` at once.
    fn spec_run(
        &mut self,
        tickets: Vec<Ticket>,
        worktree: Self::Worktree,
        base: &str,
        parallel: NonZeroUsize,
    ) -> Result<Reached, FailedRun>;
    /// Take the issue, from its Issue branch in `worktree`, through the
    /// Delivery into the Base branch `base`, opening with `opening`.
    fn deliver(
        &mut self,
        worktree: Self::Worktree,
        base: &str,
        opening: Opening,
    ) -> Result<Reached, FailedRun>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// The outside world of a Run on `issue` from the opened Launch directory
/// `directory`: GitHub, its git, the logs, the Claim it made, the worktree,
/// the Spec run and the Delivery to `goal`, with `base_fix` and the Run's
/// Session `logs`, its sessions on `harness`.
struct LaunchAndGitHub<'a> {
    directory: &'a LaunchDirectory,
    issue: &'a IssueUrl,
    goal: Goal,
    security_fix: bool,
    security_review: bool,
    base_fix: &'a mut BaseFix,
    logs: Logs,
    harness: &'a Choice,
    claim: Option<Claim<'a>>,
}

impl LaunchAndGitHub<'_> {
    /// The Delivery of the issue into the Base branch `base`.
    fn delivery<'d>(&'d mut self, base: &'d str) -> Delivery<'d> {
        Delivery {
            security_fix: self.security_fix,
            security_review: self.security_review,
            issue: self.issue,
            base,
            goal: self.goal,
            base_fix: self.base_fix,
            logs: &self.logs,
            harness: self.harness,
        }
    }
}

impl<'a> Outside for LaunchAndGitHub<'a> {
    type Worktree = Worktree;

    fn tickets(&mut self) -> Result<Vec<Ticket>> {
        GitHub::new().tickets(self.issue)
    }

    fn started(&mut self, work: Work) {
        logs::started(work, self.harness);
    }

    fn select(&mut self) -> Result<Selection> {
        branch::select(self.directory.git(), &GitHub::new(), self.issue)
    }

    fn prepare_base_branch(&mut self, named: Option<&str>, pull: bool) -> Result<String> {
        let base = self.directory.prepare_base_branch(named)?;
        if pull {
            base.pull();
        }
        Ok(base.name().to_string())
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn make_claim(&mut self) -> Result<()> {
        self.claim = Some(claim::make(self.issue, self.directory.git())?);
        Ok(())
    }

    fn end_claim(&mut self, reached: Option<Goal>) {
        if let Some(claim) = self.claim.take() {
            claim.end(reached);
        }
    }

    fn worktree(&mut self, checkout: Checkout, branch: &str, base: &str) -> Result<Worktree> {
        let (launch, repo) = (self.directory.git(), &self.issue.repo);
        match checkout {
            Checkout::Fresh => Worktree::create_fresh(launch, repo, branch, base),
            Checkout::Continuation => Worktree::continue_existing(launch, repo, branch, base),
        }
    }

    fn spec_run(
        &mut self,
        tickets: Vec<Ticket>,
        worktree: Worktree,
        base: &str,
        parallel: NonZeroUsize,
    ) -> Result<Reached, FailedRun> {
        spec_run::run(tickets, worktree, self.delivery(base), parallel)
    }

    fn deliver(
        &mut self,
        worktree: Worktree,
        base: &str,
        opening: Opening,
    ) -> Result<Reached, FailedRun> {
        let mut pull_request = crate::pull_request::PullRequest::new(
            self.issue,
            worktree.branch(),
            base,
            GitHub::new(),
        );
        self.delivery(base)
            .deliver(worktree, opening, &mut pull_request, None)
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

#[cfg(test)]
mod tests {
    use anyhow::bail;

    use super::*;
    use crate::base_fix::BaseFixAsk;
    use crate::github::{PrState, PullRequest};
    use crate::labels::Labels;
    use crate::notification::NotificationAsk;

    /// What the Run's start did outside itself, in the order it did it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        /// Read the issue's Tickets.
        Tickets,
        /// Recorded the work as started: a Spec run if true, else a Run.
        Started { spec_run: bool },
        /// Selected the Issue branch.
        Select,
        /// Prepared the named Base branch with this checkout update policy.
        PrepareBaseBranch { named: Option<String>, pull: bool },
        /// Asked whether the Run was interrupted.
        Interrupted,
        /// Made the Claim.
        MakeClaim,
        /// Ended the Claim with this goal reached, or none.
        EndClaim(Option<Goal>),
        /// Created the worktree for this Issue branch, so, on this Base
        /// branch.
        Worktree(Checkout, String, String),
        /// Handed the worktree created so to the Spec run, with this many
        /// Tickets, into this Base branch, this many at once.
        SpecRun(Checkout, usize, String, usize),
        /// Handed the worktree created so to the Delivery, into this Base
        /// branch, opening with this prompt.
        Deliver(Checkout, String, String),
        /// Wrote this progress line.
        Step(String),
    }

    /// What the outside world answers. The Issue branch is a fresh
    /// `issue-7`, the issue has no Tickets, the checked-out `main` settles
    /// the Base branch when none is named, and nothing fails or is
    /// interrupted, unless scripted otherwise.
    #[derive(Default)]
    struct Script {
        /// Whether each Ticket is open.
        tickets: Vec<bool>,
        /// The Issue branch selected.
        selection: Option<Selection>,
        base_fails: bool,
        interrupted: bool,
        claim_fails: bool,
        worktree_fails: bool,
        /// Whether the Spec run or the Delivery fails.
        work_fails: bool,
    }

    /// The outside world as `script` answers it, with each call recorded in
    /// `calls`. The Spec run and the Delivery reach `goal`.
    struct Scripted {
        script: Script,
        goal: Goal,
        calls: Vec<Call>,
    }

    impl Scripted {
        fn new(script: Script) -> Self {
            Scripted {
                script,
                goal: Goal::ReadyForReview,
                calls: Vec::new(),
            }
        }

        /// How the Spec run or the Delivery ends.
        fn work_ends(&self) -> Result<Reached, FailedRun> {
            if self.script.work_fails {
                return Err(anyhow!("the work failed").into());
            }
            Ok(Reached {
                pr_url: PR_URL.to_string(),
                goal: self.goal,
                log: None,
                ticket_lines: Vec::new(),
            })
        }
    }

    impl Outside for Scripted {
        /// How it was created.
        type Worktree = Checkout;

        fn tickets(&mut self) -> Result<Vec<Ticket>> {
            self.calls.push(Call::Tickets);
            Ok(self
                .script
                .tickets
                .iter()
                .enumerate()
                .map(|(at, &is_open)| ticket(10 + at as u64, is_open))
                .collect())
        }

        fn started(&mut self, work: Work) {
            let spec_run = match work {
                Work::Run(_) => false,
                Work::SpecRun(_) => true,
                Work::PickupRun(_) | Work::ArchitectRun(_) | Work::SecurityRun(_) => {
                    panic!("not a Run's work")
                }
            };
            self.calls.push(Call::Started { spec_run });
        }

        fn select(&mut self) -> Result<Selection> {
            self.calls.push(Call::Select);
            Ok(self.script.selection.take().unwrap_or_else(fresh))
        }

        fn prepare_base_branch(&mut self, named: Option<&str>, pull: bool) -> Result<String> {
            self.calls.push(Call::PrepareBaseBranch {
                named: named.map(String::from),
                pull,
            });
            if self.script.base_fails {
                bail!("base branch gone does not exist on origin; push it first");
            }
            Ok(named.unwrap_or(CHECKED_OUT).to_string())
        }

        fn interrupted(&mut self) -> bool {
            self.calls.push(Call::Interrupted);
            self.script.interrupted
        }

        fn make_claim(&mut self) -> Result<()> {
            self.calls.push(Call::MakeClaim);
            if self.script.claim_fails {
                bail!("could not make the Claim on #7");
            }
            Ok(())
        }

        fn end_claim(&mut self, reached: Option<Goal>) {
            self.calls.push(Call::EndClaim(reached));
        }

        fn worktree(&mut self, checkout: Checkout, branch: &str, base: &str) -> Result<Checkout> {
            self.calls.push(Call::Worktree(
                checkout,
                branch.to_string(),
                base.to_string(),
            ));
            if self.script.worktree_fails {
                bail!("could not create the worktree");
            }
            Ok(checkout)
        }

        fn spec_run(
            &mut self,
            tickets: Vec<Ticket>,
            worktree: Checkout,
            base: &str,
            parallel: NonZeroUsize,
        ) -> Result<Reached, FailedRun> {
            self.calls.push(Call::SpecRun(
                worktree,
                tickets.len(),
                base.to_string(),
                parallel.get(),
            ));
            self.work_ends()
        }

        fn deliver(
            &mut self,
            worktree: Checkout,
            base: &str,
            opening: Opening,
        ) -> Result<Reached, FailedRun> {
            assert_eq!(opening.kind, IMPLEMENT);
            assert!(!opening.catch_up_from_origin);
            self.calls
                .push(Call::Deliver(worktree, base.to_string(), opening.prompt));
            self.work_ends()
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
    }

    const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

    /// The branch checked out in the Launch directory.
    const CHECKED_OUT: &str = "main";

    fn seven() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
    }

    fn ticket(number: u64, is_open: bool) -> Ticket {
        Ticket {
            number,
            is_open,
            labels: Labels::default(),
            has_sub_issues: false,
            blockers: Vec::new(),
            open_blockers: Vec::new(),
        }
    }

    fn fresh() -> Selection {
        Selection::Fresh {
            branch: "issue-7".to_string(),
        }
    }

    /// A Continuation of `issue-7`, with its open PR into `pr_base` if any.
    fn continued(pr_base: Option<&str>) -> Selection {
        Selection::Continuation {
            branch: "issue-7".to_string(),
            pr: pr_base.map(|base| PullRequest {
                number: 1,
                url: PR_URL.to_string(),
                state: PrState::Open,
                head: "issue-7".to_string(),
                base: base.to_string(),
                is_draft: false,
            }),
        }
    }

    /// What a Run started by its command is asked, by default.
    fn asks() -> Asks {
        Asks {
            security_fix: false,
            security_review: false,
            goal: Goal::ReadyForReview,
            notification: NotificationAsk::Skip,
            tickets_at_once: NonZeroUsize::new(1).unwrap(),
            parallel_asked: false,
            base_fix: BaseFixAsk::Forbid,
            launch_pull: false,
            harness: Choice::default(),
        }
    }

    /// Start the Run on issue #7, asked `asks` and started by `started_by`,
    /// against `outside`, with `main` checked out.
    fn start_by(
        outside: &mut Scripted,
        asks: &Asks,
        started_by: StartedBy,
    ) -> Result<Reached, FailedRun> {
        start(outside, &seven(), asks, started_by, Some(CHECKED_OUT))
    }

    /// The calls of a Run its command started on `script`, and how it ended.
    fn run_on(script: Script) -> (Vec<Call>, Result<Reached, FailedRun>) {
        run_asked(script, &asks())
    }

    /// [`run_on`], asked `asks`.
    fn run_asked(script: Script, asks: &Asks) -> (Vec<Call>, Result<Reached, FailedRun>) {
        let mut outside = Scripted::new(script);
        let ended = start_by(&mut outside, asks, StartedBy::Command);
        (outside.calls, ended)
    }

    /// The cause a Run failed with.
    fn cause(ended: Result<Reached, FailedRun>) -> String {
        match ended {
            Ok(_) => panic!("the Run reached its goal"),
            Err(failed) => format!("{:#}", failed.error),
        }
    }

    fn base_branch(named: Option<&str>) -> Call {
        Call::PrepareBaseBranch {
            named: named.map(String::from),
            pull: false,
        }
    }

    fn worktree(checkout: Checkout, base: &str) -> Call {
        Call::Worktree(checkout, "issue-7".to_string(), base.to_string())
    }

    /// The Delivery of a fresh `issue-7` into `base`, with the fresh prompt.
    fn deliver_fresh(base: &str) -> Call {
        Call::Deliver(
            Checkout::Fresh,
            base.to_string(),
            prompt::fresh(&seven(), base, "issue-7"),
        )
    }

    /// Assert that `calls` made `in_turn`, one straight after another.
    fn assert_in_turn(calls: &[Call], in_turn: &[Call]) {
        assert!(
            calls.windows(in_turn.len()).any(|window| window == in_turn),
            "{in_turn:?} not in turn in {calls:?}"
        );
    }

    fn made_claim_or_worktree(calls: &[Call]) -> bool {
        calls
            .iter()
            .any(|call| matches!(call, Call::MakeClaim | Call::Worktree(..)))
    }

    // What started the Run.

    #[test]
    fn a_child_run_reads_no_tickets_and_makes_no_claim() {
        let kind = Kind::Ticket {
            spec_branch: "issue-3".to_string(),
        };
        let mut outside = Scripted::new(Script::default());

        let ended = start_by(&mut outside, &asks(), StartedBy::Child(&kind));

        assert!(ended.is_ok());
        assert_eq!(
            outside.calls,
            [
                Call::Started { spec_run: false },
                Call::Select,
                base_branch(Some("issue-3")),
                Call::Interrupted,
                worktree(Checkout::Fresh, "issue-3"),
                deliver_fresh("issue-3"),
            ]
        );
    }

    #[test]
    fn a_run_its_command_started_makes_the_claim_before_the_worktree_and_ends_it() {
        let (calls, ended) = run_on(Script::default());

        assert_eq!(
            ended.ok().map(|reached| reached.pr_url),
            Some(PR_URL.to_string())
        );
        assert_eq!(
            calls,
            [
                Call::Tickets,
                Call::Started { spec_run: false },
                Call::Select,
                base_branch(None),
                Call::Interrupted,
                Call::MakeClaim,
                worktree(Checkout::Fresh, "main"),
                deliver_fresh("main"),
                Call::EndClaim(Some(Goal::ReadyForReview)),
            ]
        );
    }

    #[test]
    fn a_dispatched_run_makes_the_claim_and_ends_it() {
        let mut outside = Scripted::new(Script::default());

        let ended = start_by(
            &mut outside,
            &asks(),
            StartedBy::Dispatch { base: "develop" },
        );

        assert!(ended.is_ok());
        assert_eq!(
            outside.calls,
            [
                Call::Tickets,
                Call::Started { spec_run: false },
                Call::Select,
                base_branch(Some("develop")),
                Call::Interrupted,
                Call::MakeClaim,
                worktree(Checkout::Fresh, "develop"),
                deliver_fresh("develop"),
                Call::EndClaim(Some(Goal::ReadyForReview)),
            ]
        );
    }

    // The work recorded as started, and the refusals.

    #[test]
    fn parallel_on_an_issue_with_no_sub_issues_is_recorded_as_a_run_and_fails_before_selection() {
        let asks = Asks {
            parallel_asked: true,
            ..asks()
        };

        let (calls, ended) = run_asked(Script::default(), &asks);

        assert_eq!(
            cause(ended),
            "parallel is only for a Spec, and #7 has no sub-issues"
        );
        assert_eq!(calls, [Call::Tickets, Call::Started { spec_run: false }]);
    }

    #[test]
    fn a_fresh_spec_with_every_ticket_closed_is_recorded_as_a_spec_run_and_fails_before_its_base() {
        let (calls, ended) = run_on(Script {
            tickets: vec![false, false],
            ..Script::default()
        });

        assert_eq!(
            cause(ended),
            "every Ticket is closed and there is no Spec branch; nothing to do"
        );
        assert_eq!(
            calls,
            [
                Call::Tickets,
                Call::Started { spec_run: true },
                Call::Select
            ]
        );
    }

    #[test]
    fn neither_refusal_makes_a_claim_or_a_worktree() {
        let parallel = Asks {
            parallel_asked: true,
            ..asks()
        };
        let all_closed = Script {
            tickets: vec![false],
            ..Script::default()
        };

        for (calls, ended) in [run_asked(Script::default(), &parallel), run_on(all_closed)] {
            assert!(ended.is_err());
            assert!(!made_claim_or_worktree(&calls), "{calls:?}");
        }
    }

    #[test]
    fn a_continuation_of_a_spec_with_every_ticket_closed_goes_on_to_the_spec_run() {
        let (calls, ended) = run_on(Script {
            tickets: vec![false, false],
            selection: Some(continued(None)),
            ..Script::default()
        });

        assert!(ended.is_ok());
        assert!(calls.contains(&Call::SpecRun(
            Checkout::Continuation,
            2,
            "main".to_string(),
            1
        )));
    }

    #[test]
    fn an_issue_with_no_sub_issues_is_never_refused_as_all_closed() {
        let (calls, ended) = run_on(Script::default());

        assert!(ended.is_ok());
        assert!(calls.contains(&deliver_fresh("main")));
    }

    // The Base branch.

    #[test]
    fn a_continuations_open_pr_names_the_base_branch_over_the_one_given() {
        let mut outside = Scripted::new(Script {
            selection: Some(continued(Some("release"))),
            ..Script::default()
        });

        let ended = start_by(
            &mut outside,
            &asks(),
            StartedBy::Dispatch { base: "develop" },
        );

        assert!(ended.is_ok());
        let line = format!(
            "continuing issue-7 and its PR {PR_URL}, so the Base branch is release, \
             not the given develop"
        );
        assert_in_turn(
            &outside.calls,
            &[Call::Select, Call::Step(line), base_branch(Some("release"))],
        );
    }

    #[test]
    fn a_continuations_open_pr_names_the_base_branch_over_the_one_checked_out() {
        let (calls, _) = run_on(Script {
            selection: Some(continued(Some("release"))),
            ..Script::default()
        });

        let line = format!(
            "continuing issue-7 and its PR {PR_URL}, so the Base branch is release, \
             not the checked-out main"
        );
        assert_in_turn(
            &calls,
            &[Call::Select, Call::Step(line), base_branch(Some("release"))],
        );
    }

    #[test]
    fn an_open_pr_into_the_branch_checked_out_says_nothing_of_it() {
        let (calls, _) = run_on(Script {
            selection: Some(continued(Some("main"))),
            ..Script::default()
        });

        assert_in_turn(&calls, &[Call::Select, base_branch(Some("main"))]);
    }

    #[test]
    fn without_an_open_pr_the_branch_given_names_the_base_branch() {
        let mut outside = Scripted::new(Script {
            selection: Some(continued(None)),
            ..Script::default()
        });

        let ended = start_by(
            &mut outside,
            &asks(),
            StartedBy::Dispatch { base: "develop" },
        );

        assert!(ended.is_ok());
        assert_in_turn(
            &outside.calls,
            &[Call::Select, base_branch(Some("develop"))],
        );
    }

    #[test]
    fn with_nothing_given_no_branch_is_named_and_the_checked_out_branch_settles_it() {
        let (calls, _) = run_on(Script::default());

        assert_in_turn(&calls, &[Call::Select, base_branch(None)]);
        assert!(calls.contains(&worktree(Checkout::Fresh, CHECKED_OUT)));
    }

    #[test]
    fn a_base_branch_that_cannot_be_settled_fails_before_the_claim() {
        let (calls, ended) = run_on(Script {
            base_fails: true,
            ..Script::default()
        });

        assert_eq!(
            cause(ended),
            "base branch gone does not exist on origin; push it first"
        );
        assert_eq!(calls.last(), Some(&base_branch(None)));
    }

    // The pull.

    #[test]
    fn the_pull_asked_for_pulls_the_settled_base_branch_before_the_claim() {
        let asks = Asks {
            launch_pull: true,
            ..asks()
        };
        let mut outside = Scripted::new(Script::default());

        let ended = start_by(&mut outside, &asks, StartedBy::Dispatch { base: "develop" });

        assert!(ended.is_ok());
        assert_in_turn(
            &outside.calls,
            &[
                Call::PrepareBaseBranch {
                    named: Some("develop".to_string()),
                    pull: true,
                },
                Call::Interrupted,
                Call::MakeClaim,
            ],
        );
    }

    #[test]
    fn a_continuing_spec_prepares_its_open_prs_base_with_pull_before_the_claim() {
        let asks = Asks {
            launch_pull: true,
            ..asks()
        };
        let mut outside = Scripted::new(Script {
            tickets: vec![true],
            selection: Some(continued(Some("release"))),
            ..Script::default()
        });

        let ended = start_by(&mut outside, &asks, StartedBy::Dispatch { base: "develop" });

        assert!(ended.is_ok());
        assert_in_turn(
            &outside.calls,
            &[
                Call::PrepareBaseBranch {
                    named: Some("release".to_string()),
                    pull: true,
                },
                Call::Interrupted,
                Call::MakeClaim,
                worktree(Checkout::Continuation, "release"),
                Call::SpecRun(Checkout::Continuation, 1, "release".to_string(), 1),
            ],
        );
    }

    #[test]
    fn without_the_pull_nothing_is_pulled() {
        let (calls, _) = run_on(Script::default());

        assert!(calls.contains(&base_branch(None)));
    }

    // The interrupt.

    #[test]
    fn an_interrupt_before_the_claim_fails_the_run_as_interrupted() {
        let (calls, ended) = run_on(Script {
            interrupted: true,
            ..Script::default()
        });

        let failed = ended.err().expect("the Run reached its goal");
        assert!(failed.interrupted);
        assert_eq!(failed.error.to_string(), "interrupted");
        assert_eq!(calls.last(), Some(&Call::Interrupted));
        assert!(!made_claim_or_worktree(&calls), "{calls:?}");
    }

    // The Claim.

    #[test]
    fn a_claim_that_cannot_be_made_fails_the_run_before_the_worktree() {
        let (calls, ended) = run_on(Script {
            claim_fails: true,
            ..Script::default()
        });

        assert_eq!(cause(ended), "could not make the Claim on #7");
        assert_eq!(calls.last(), Some(&Call::MakeClaim));
    }

    #[test]
    fn the_claim_is_ended_with_the_goal_the_run_reached() {
        let mut outside = Scripted::new(Script::default());
        outside.goal = Goal::Merged;

        let ended = start_by(&mut outside, &asks(), StartedBy::Command);

        assert!(ended.is_ok());
        assert_eq!(
            outside.calls.last(),
            Some(&Call::EndClaim(Some(Goal::Merged)))
        );
    }

    #[test]
    fn the_claim_is_ended_with_none_when_the_worktree_cannot_be_created() {
        let (calls, ended) = run_on(Script {
            worktree_fails: true,
            ..Script::default()
        });

        assert_eq!(cause(ended), "could not create the worktree");
        assert_eq!(
            calls[calls.len() - 2..],
            [worktree(Checkout::Fresh, "main"), Call::EndClaim(None)]
        );
    }

    #[test]
    fn the_claim_is_ended_with_none_when_the_delivery_fails() {
        let (calls, ended) = run_on(Script {
            work_fails: true,
            ..Script::default()
        });

        assert!(ended.is_err());
        assert_eq!(
            calls[calls.len() - 2..],
            [deliver_fresh("main"), Call::EndClaim(None)]
        );
    }

    #[test]
    fn the_claim_is_ended_with_none_when_the_spec_run_fails() {
        let (calls, ended) = run_on(Script {
            tickets: vec![true],
            work_fails: true,
            ..Script::default()
        });

        assert!(ended.is_err());
        assert_eq!(
            calls[calls.len() - 2..],
            [
                Call::SpecRun(Checkout::Fresh, 1, "main".to_string(), 1),
                Call::EndClaim(None),
            ]
        );
    }

    // The hand-off.

    #[test]
    fn a_fresh_selection_creates_a_fresh_worktree_and_opens_with_the_fresh_prompt() {
        let (calls, _) = run_on(Script::default());

        assert_in_turn(
            &calls,
            &[worktree(Checkout::Fresh, "main"), deliver_fresh("main")],
        );
    }

    #[test]
    fn a_continuation_continues_the_worktree_and_opens_with_the_continuation_prompt() {
        for pr_base in [None, Some("main")] {
            let (calls, _) = run_on(Script {
                selection: Some(continued(pr_base)),
                ..Script::default()
            });

            let pr_url = pr_base.map(|_| PR_URL);
            let prompt = prompt::continuation(&seven(), "main", "issue-7", pr_url);
            assert_in_turn(
                &calls,
                &[
                    worktree(Checkout::Continuation, "main"),
                    Call::Deliver(Checkout::Continuation, "main".to_string(), prompt),
                ],
            );
        }
    }

    #[test]
    fn tickets_hand_the_worktree_to_the_spec_run_with_the_number_to_run_at_once() {
        let asks = Asks {
            tickets_at_once: NonZeroUsize::new(3).unwrap(),
            ..asks()
        };

        let (calls, ended) = run_asked(
            Script {
                tickets: vec![true, false],
                ..Script::default()
            },
            &asks,
        );

        assert!(ended.is_ok());
        assert!(calls.contains(&Call::Started { spec_run: true }));
        assert_in_turn(
            &calls,
            &[
                worktree(Checkout::Fresh, "main"),
                Call::SpecRun(Checkout::Fresh, 2, "main".to_string(), 3),
            ],
        );
    }
}
