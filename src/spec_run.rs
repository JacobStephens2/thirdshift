//! A Spec run: each of a Spec's Tickets, in dependency order and several at
//! once, taken by a Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch, kept mergeable and green like a Run's PR, and Self-merged
//! when the Spec run was asked to merge.
//!
//! Its lifecycle reaches the child Runs, GitHub, the Spec PR, the Delivery,
//! the interrupt and the progress lines only through [`Outside`]:
//! [`ChildRunsAndGitHub`] does that for real; `Scripted`, in tests, from a
//! script, recording each call.

mod spec_pr;
mod ticket_board;

use std::num::NonZeroUsize;

use anyhow::{Result, bail};

use crate::base_fix::BaseFixAsk;
use crate::child_run::{Ended, Kind, Runs};
use crate::delivery::{Delivery, Opening};
use crate::failed_run::{FailedRun, interrupted_or};
use crate::github::{GitHub, Ticket};
use crate::harness::Choice;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::prompt;
use crate::run::Reached;
use crate::worktree::Worktree;

use spec_pr::SpecPr;
use ticket_board::TicketBoard;

/// The Spec review session's kind, in its progress lines and log name.
const SPEC_REVIEW: &str = "spec-review";

/// Whether an issue has Tickets, as a Spec has, and every one is closed:
/// `open` says of each of its sub-issues whether it is open.
pub fn all_closed(open: impl IntoIterator<Item = bool>) -> bool {
    let mut open = open.into_iter().peekable();
    open.peek().is_some() && open.all(|is_open| !is_open)
}

/// Take the Spec, `delivery`'s issue, whose Tickets were last read as
/// `tickets`, from its Spec branch, checked out in `worktree`, to a Spec PR
/// into `delivery`'s Base branch that reaches its goal, as [`run_through`]
/// says, once the Spec PR, if it is open, is turned back into a draft. The
/// worktree is cleaned up when this returns, or kept by the Failed run path
/// if its work did not reach origin. If `delivery`'s Base fix may start one,
/// so may each Ticket's Run, into the Spec branch.
pub fn run(
    tickets: Vec<Ticket>,
    worktree: Worktree,
    delivery: Delivery,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let (spec, base) = (delivery.issue, delivery.base);
    let branch = worktree.branch().to_string();
    let spec_pr = SpecPr::resume(spec, &branch, base)?;
    let mut outside = ChildRunsAndGitHub::new(worktree, delivery, spec_pr);
    run_through(&mut outside, spec, base, &branch, tickets, parallel)
}

/// Take `spec`, whose Tickets were last read as `tickets`, from its Spec
/// branch `branch` to a Spec PR into `base` that reaches its goal, all
/// through `outside`. The Spec branch is pushed before any Ticket starts,
/// and up to `parallel` Tickets run at once, the graph read again whenever
/// one ends. The Spec PR is opened as a draft once the first Ticket lands,
/// and its Tickets checklist rewritten as Tickets start and end. A Ticket
/// that fails stops only the Tickets it blocks; with any Ticket not done,
/// this is a Failed spec run, with the Spec PR's URL if it is open, which
/// leaves it a draft. An interrupt ends it too, once the running Tickets'
/// Runs have ended, starting nothing more. Once every Ticket has landed,
/// the Spec PR is told so, and taken to its goal by the Delivery, opening
/// with the Spec review on the Spec branch caught up from origin, and the
/// Tickets checklist put back in the Spec PR's body before it is marked
/// ready, and shown again if the Delivery fails. However it ends, it carries
/// a line on each Ticket it landed or did not get done, for the Run
/// notification.
fn run_through(
    outside: &mut impl Outside,
    spec: &IssueUrl,
    base: &str,
    branch: &str,
    tickets: Vec<Ticket>,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let (ticket_lines, landed) = land_tickets(outside, tickets, parallel);
    // Every Ticket landed only once the Spec PR is told so.
    let landed = landed.and_then(|checklist| {
        let spec_pr_url = outside.landed(&checklist)?;
        Ok((checklist, spec_pr_url))
    });
    let ended = match landed {
        Ok((checklist, spec_pr_url)) => {
            review_and_deliver(outside, spec, base, branch, &checklist, &spec_pr_url)
        }
        Err(error) => {
            let interrupted = outside.interrupted();
            Err(FailedRun {
                error: interrupted_or(error, interrupted),
                pr_url: outside.spec_pr_url(),
                log: None,
                interrupted,
                ticket_lines: Vec::new(),
            })
        }
    };
    match ended {
        Ok(reached) => Ok(Reached {
            ticket_lines,
            ..reached
        }),
        Err(failed) => Err(FailedRun {
            ticket_lines,
            ..failed
        }),
    }
}

/// Once every Ticket has landed, with `checklist`, and the Spec PR,
/// `spec_pr_url`, is told so: take it to its goal by the Delivery, opening
/// with the Spec review, all through `outside`, as [`run_through`] does.
fn review_and_deliver(
    outside: &mut impl Outside,
    spec: &IssueUrl,
    base: &str,
    branch: &str,
    checklist: &str,
    spec_pr_url: &str,
) -> Result<Reached, FailedRun> {
    if outside.interrupted() {
        return Err(FailedRun {
            interrupted: true,
            pr_url: Some(spec_pr_url.to_string()),
            ..FailedRun::from(anyhow::anyhow!("interrupted"))
        });
    }
    let opening = Opening {
        kind: SPEC_REVIEW,
        prompt: prompt::spec_review(spec, base, branch, spec_pr_url),
        catch_up_from_origin: true,
    };
    // The Spec review may have rewritten the body without the checklist.
    let delivered = outside.deliver(opening, |outside| outside.put_back(checklist));
    if delivered.is_err() {
        outside.show(checklist);
    }
    delivered
}

/// What the Spec run does or reads outside itself: the push of the Spec
/// branch, the Tickets' child Runs, the graph read again, the Spec PR, the
/// Delivery, the interrupt and its progress lines.
trait Outside {
    /// Push the Spec branch to origin.
    fn push(&mut self) -> Result<()>;
    /// Start Ticket `number`'s Run, a Merge run into the Spec branch.
    fn start_ticket(&mut self, number: u64) -> Result<()>;
    /// Wait for the next running Ticket's Run to end: its number and how it
    /// ended.
    fn next_ending(&mut self) -> (u64, Result<Ended>);
    /// The Spec's Tickets, read again.
    fn tickets(&mut self) -> Result<Vec<Ticket>>;
    /// Show `checklist` as the Spec PR's Tickets checklist, if it is open.
    fn show(&mut self, checklist: &str);
    /// A Ticket landed: open the Spec PR as a draft with `checklist`, or
    /// show it. Returns the Spec PR's URL.
    fn landed(&mut self, checklist: &str) -> Result<String>;
    /// The Spec PR's URL, if it is open.
    fn spec_pr_url(&mut self) -> Option<String>;
    /// Put `checklist` back in the Spec PR's body.
    fn put_back(&mut self, checklist: &str) -> Result<()>;
    /// Take the Spec PR to its goal by the Delivery, opening with `opening`,
    /// running `before_ready` before the Spec PR is marked ready. Called at
    /// most once, after the Ticket loop.
    fn deliver(
        &mut self,
        opening: Opening,
        before_ready: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Reached, FailedRun>;
    /// Whether the Spec run was interrupted.
    fn interrupted(&mut self) -> bool;
    /// Write the progress line `line`.
    fn step(&mut self, line: String);
}

/// The outside world of a Spec run on `delivery`'s Spec, from its Spec
/// branch checked out in `worktree`: child `thirdshift` Runs, each of which
/// may start a Base fix if `base_fix` allows one, and runs its sessions on
/// `harness`, owned together in `children`, GitHub, the Spec PR
/// `spec_pr`, and `delivery`, which takes the
/// worktree when it starts.
struct ChildRunsAndGitHub<'a> {
    spec: &'a IssueUrl,
    /// Fields drop in declaration order: stop and drain child salvage before
    /// Worktree cleanup, including when the Spec run owner unwinds.
    children: Runs,
    /// Held until the Delivery starts, which takes it.
    worktree: Option<Worktree>,
    /// Held until it starts.
    delivery: Option<Delivery<'a>>,
    base_fix: BaseFixAsk,
    harness: &'a Choice,
    spec_pr: SpecPr<'a>,
}

impl<'a> ChildRunsAndGitHub<'a> {
    /// The outside world of a Spec run, as the struct says, with no Ticket's
    /// Run started yet.
    fn new(worktree: Worktree, delivery: Delivery<'a>, spec_pr: SpecPr<'a>) -> Self {
        Self {
            spec: delivery.issue,
            children: Runs::default(),
            worktree: Some(worktree),
            base_fix: delivery.base_fix.ask_of_tickets(),
            harness: delivery.harness,
            delivery: Some(delivery),
            spec_pr,
        }
    }

    /// The worktree, before the Delivery takes it.
    fn worktree(&self) -> &Worktree {
        self.worktree
            .as_ref()
            .expect("the Ticket loop runs before the Delivery")
    }
}

impl Outside for ChildRunsAndGitHub<'_> {
    fn push(&mut self) -> Result<()> {
        self.worktree().push()
    }

    fn start_ticket(&mut self, number: u64) -> Result<()> {
        let kind = Kind::Ticket {
            spec_branch: self.worktree().branch().to_string(),
        };
        let ticket = self.spec.sibling(number);
        self.children
            .start(&ticket, kind, self.base_fix.clone(), self.harness)
    }

    fn next_ending(&mut self) -> (u64, Result<Ended>) {
        let (number, ending) = self
            .children
            .next_ending()
            .expect("the Ticket board has an outstanding Run");
        if let Ok(ended) = &ending {
            progress::step(match ended {
                Ended::Reached { .. } => format!("#{number} landed"),
                Ended::Interrupted => format!("#{number} interrupted"),
                Ended::Failed { .. } => format!("#{number} failed"),
            });
        }
        (number, ending)
    }

    fn tickets(&mut self) -> Result<Vec<Ticket>> {
        GitHub::new().completion().tickets(self.spec)
    }

    fn show(&mut self, checklist: &str) {
        self.spec_pr.show(checklist);
    }

    fn landed(&mut self, checklist: &str) -> Result<String> {
        self.spec_pr.landed(checklist).map(str::to_string)
    }

    fn spec_pr_url(&mut self) -> Option<String> {
        self.spec_pr.url().map(str::to_string)
    }

    fn put_back(&mut self, checklist: &str) -> Result<()> {
        self.spec_pr.put_back(checklist)
    }

    /// Hands the worktree and the Delivery over, then lends `self` to
    /// `before_ready`, as the Delivery no longer borrows it.
    fn deliver(
        &mut self,
        opening: Opening,
        before_ready: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Reached, FailedRun> {
        let worktree = self.worktree.take().expect("the Delivery starts once");
        let delivery = self.delivery.take().expect("the Delivery starts once");
        delivery.deliver(worktree, opening, || before_ready(self))
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

/// Push the Spec branch, then keep up to `parallel` of the Tickets, last
/// read as `tickets`, running, each time one ends starting ready ones,
/// lowest number first, until none is ready and none is running, showing
/// the Tickets checklist on the Spec PR as Tickets start and end, and
/// telling it when a Ticket lands, all through `outside`. Returns a line on
/// each Ticket that landed or is not done, none if the Spec branch could not
/// be pushed, and the last Tickets checklist if every Ticket is done: if
/// not, it fails, after putting those lines on stderr unless an interrupt or
/// another error ended it first.
fn land_tickets(
    outside: &mut impl Outside,
    tickets: Vec<Ticket>,
    parallel: NonZeroUsize,
) -> (Vec<String>, Result<String>) {
    if let Err(error) = outside.push() {
        return (Vec::new(), Err(error));
    }
    let mut board = TicketBoard::new(tickets, parallel);
    let landing = run_ready_tickets(outside, &mut board);
    let lines = board.lines();
    let landed = landing.and_then(|()| {
        let not_done: Vec<String> = board
            .not_done()
            .iter()
            .map(|number| format!("#{number}"))
            .collect();
        if not_done.is_empty() {
            return Ok(board.checklist());
        }
        for line in &lines {
            outside.step(line.clone());
        }
        bail!("Tickets not done: {}", not_done.join(", "));
    });
    (lines, landed)
}

/// Keep the Tickets on `board` running as it offers them, until it offers
/// none and none is running, telling it as each starts and ends and as the
/// graph is read again, and showing the Spec PR the Tickets checklist as
/// they start and end, and telling it when one lands, all through
/// `outside`. An interrupt, or an error other than a Ticket failing, stops
/// any more from starting, and fails this once those running have ended.
fn run_ready_tickets(outside: &mut impl Outside, board: &mut TicketBoard) -> Result<()> {
    let mut error = None;
    loop {
        let mut started = false;
        loop {
            if error.is_some() || outside.interrupted() {
                board.stop();
            }
            let Some(ticket) = board.next_to_start() else {
                break;
            };
            outside.step(format!("starting #{ticket}"));
            match outside.start_ticket(ticket) {
                Ok(()) => {
                    board.started(ticket);
                    started = true;
                }
                Err(start_error) => error = Some(start_error),
            }
        }
        if started {
            outside.show(&board.checklist());
        }
        if !board.any_running() {
            break;
        }
        let (ticket, result) = outside.next_ending();
        // Once there is an error, the rest are only waited for, but each
        // still has its line in the checklist, from the last graph read.
        let ending = result.unwrap_or_else(|finish_error| {
            let cause = format!("{finish_error:#}");
            error.get_or_insert(finish_error);
            Ended::Failed { cause, log: None }
        });
        let landed = matches!(ending, Ended::Reached { .. });
        board.ended(ticket, ending);
        match outside.tickets() {
            Ok(reread) => board.reread(reread),
            Err(reread_error) => {
                error.get_or_insert(reread_error);
            }
        }
        let list = board.checklist();
        if landed {
            if let Err(open_error) = outside.landed(&list) {
                error.get_or_insert(open_error);
            }
        } else {
            outside.show(&list);
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    if outside.interrupted() {
        bail!("interrupted");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::PathBuf;

    use anyhow::anyhow;

    use super::*;
    use crate::run::Goal;

    /// The Spec PR's URL, once it is open.
    const SPEC_PR: &str = "https://github.com/acme/widgets/pull/2";

    /// The Delivery's Session log.
    const DELIVERY_LOG: &str = "/logs/20-spec-review.jsonl";

    /// What the Spec run did outside itself, in the order it did it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Did {
        /// Pushed the Spec branch.
        Push,
        /// Started this Ticket's Run, or tried to.
        Start(u64),
        /// Received that this Ticket's Run ended.
        Ended(u64),
        /// Read the Spec's Tickets again.
        ReadTickets,
        /// Showed this checklist on the Spec PR.
        Show(String),
        /// Told the Spec PR a Ticket landed, with this checklist.
        Landed(String),
        /// Put this checklist back in the Spec PR's body.
        PutBack(String),
        /// Started the Delivery, opening with the session of this kind and
        /// prompt, caught up from origin first if `catch_up`.
        Deliver {
            kind: String,
            prompt: String,
            catch_up: bool,
        },
        /// The Delivery marked the Spec PR ready.
        Ready,
        /// Wrote this progress line.
        Line(String),
    }

    /// A Ticket as GitHub has it: its open blockers follow from the graph.
    struct Node {
        number: u64,
        is_open: bool,
        labels: Vec<&'static str>,
        blockers: Vec<u64>,
    }

    /// How a read of the Spec's Tickets goes.
    enum Read {
        /// It reads the graph as it stands.
        Graph,
        /// It reads the graph once this has changed it.
        Changes(fn(&mut Vec<Node>)),
        /// It fails with this error.
        Fails(&'static str),
    }

    /// What the outside world answers, call by call. The push, each start,
    /// each read and each Spec PR opening work, and the Run started first of
    /// those running reaches its goal, unless scripted otherwise.
    #[derive(Default)]
    struct Script {
        /// The Spec's Tickets.
        tickets: Vec<Node>,
        /// The issues outside the Spec that are open.
        open_outside: Vec<u64>,
        /// Whether the push of the Spec branch fails.
        push_fails: bool,
        /// The Tickets whose Runs cannot be started.
        start_fails: Vec<u64>,
        /// Which Run ends next, and how, in order.
        endings: VecDeque<(u64, Result<Ended>)>,
        /// How each read of the Spec's Tickets goes, in order.
        reads: VecDeque<Read>,
        /// Whether opening the Spec PR, once a Ticket lands, fails.
        landed_fails: bool,
        /// Whether the Spec PR is open before any Ticket lands.
        spec_pr_open: bool,
        /// Whether putting the checklist back fails.
        put_back_fails: bool,
        /// Whether the Delivery's opening session fails, before the step
        /// before ready.
        review_fails: bool,
        /// Whether the Delivery fails once the Spec PR is ready.
        delivery_fails: bool,
        /// How many times the Spec run is found not interrupted before it is.
        interrupted_after: Option<usize>,
    }

    /// The outside world as `script` answers it, with what the Spec run did
    /// there recorded in `did`. A Ticket whose Run reached its goal is
    /// closed, as its Run's Self-merge closes it on GitHub, and the Spec PR
    /// is open once opening it works.
    struct Scripted {
        script: Script,
        did: Vec<Did>,
        /// The Tickets whose Runs are running, in the order they started.
        running: Vec<u64>,
        /// How many times the Spec run was found not interrupted.
        not_interrupted: usize,
        /// Whether the Spec PR is open.
        spec_pr_open: bool,
    }

    impl Scripted {
        fn new(script: Script) -> Self {
            Scripted {
                spec_pr_open: script.spec_pr_open,
                script,
                did: Vec::new(),
                running: Vec::new(),
                not_interrupted: 0,
            }
        }

        /// The Spec's Tickets as GitHub has them now.
        fn graph(&self) -> Vec<Ticket> {
            let is_open = |number: u64| match self
                .script
                .tickets
                .iter()
                .find(|node| node.number == number)
            {
                Some(node) => node.is_open,
                None => self.script.open_outside.contains(&number),
            };
            self.script
                .tickets
                .iter()
                .map(|node| Ticket {
                    number: node.number,
                    is_open: node.is_open,
                    labels: node.labels.iter().copied().collect(),
                    has_sub_issues: false,
                    blockers: node.blockers.clone(),
                    open_blockers: node
                        .blockers
                        .iter()
                        .copied()
                        .filter(|blocker| is_open(*blocker))
                        .collect(),
                })
                .collect()
        }

        /// What the Spec run did that is one of `kinds`.
        fn did_only(&self, kinds: fn(&Did) -> bool) -> Vec<Did> {
            self.did.iter().filter(|did| kinds(did)).cloned().collect()
        }
    }

    impl Outside for Scripted {
        fn push(&mut self) -> Result<()> {
            self.did.push(Did::Push);
            if self.script.push_fails {
                bail!("git push origin issue-20 failed");
            }
            Ok(())
        }

        fn start_ticket(&mut self, number: u64) -> Result<()> {
            self.did.push(Did::Start(number));
            if self.script.start_fails.contains(&number) {
                bail!("could not start #{number}'s Run");
            }
            self.running.push(number);
            Ok(())
        }

        fn next_ending(&mut self) -> (u64, Result<Ended>) {
            let (number, ended) = self.script.endings.pop_front().unwrap_or_else(|| {
                let first = *self.running.first().expect("no Ticket's Run is running");
                (first, Ok(reached()))
            });
            let at = self
                .running
                .iter()
                .position(|running| *running == number)
                .unwrap_or_else(|| panic!("#{number} ends, but its Run is not running"));
            self.running.remove(at);
            self.did.push(Did::Ended(number));
            if let Ok(Ended::Reached { .. }) = ended {
                let node = self
                    .script
                    .tickets
                    .iter_mut()
                    .find(|node| node.number == number);
                node.expect("a Ticket of the Spec").is_open = false;
            }
            (number, ended)
        }

        fn tickets(&mut self) -> Result<Vec<Ticket>> {
            self.did.push(Did::ReadTickets);
            match self.script.reads.pop_front().unwrap_or(Read::Graph) {
                Read::Graph => {}
                Read::Changes(change) => change(&mut self.script.tickets),
                Read::Fails(error) => return Err(anyhow!(error)),
            }
            Ok(self.graph())
        }

        fn show(&mut self, checklist: &str) {
            self.did.push(Did::Show(checklist.to_string()));
        }

        fn landed(&mut self, checklist: &str) -> Result<String> {
            self.did.push(Did::Landed(checklist.to_string()));
            if self.script.landed_fails {
                bail!("could not open the Spec PR");
            }
            self.spec_pr_open = true;
            Ok(SPEC_PR.to_string())
        }

        fn spec_pr_url(&mut self) -> Option<String> {
            self.spec_pr_open.then(|| SPEC_PR.to_string())
        }

        fn put_back(&mut self, checklist: &str) -> Result<()> {
            self.did.push(Did::PutBack(checklist.to_string()));
            if self.script.put_back_fails {
                bail!("could not put the Tickets checklist back");
            }
            Ok(())
        }

        /// Runs `before_ready`, then marks the Spec PR ready, as the
        /// Delivery does, failing as the Failed run path would.
        fn deliver(
            &mut self,
            opening: Opening,
            before_ready: impl FnOnce(&mut Self) -> Result<()>,
        ) -> Result<Reached, FailedRun> {
            self.did.push(Did::Deliver {
                kind: opening.kind.to_string(),
                prompt: opening.prompt,
                catch_up: opening.catch_up_from_origin,
            });
            let delivered = if self.script.review_fails {
                Err(anyhow!("the spec-review session failed"))
            } else {
                before_ready(self)
            };
            let delivered = delivered.and_then(|()| {
                self.did.push(Did::Ready);
                if self.script.delivery_fails {
                    bail!("checks failed on the Spec PR");
                }
                Ok(())
            });
            match delivered {
                Ok(()) => Ok(Reached {
                    pr_url: SPEC_PR.to_string(),
                    goal: Goal::ReadyForReview,
                    log: Some(PathBuf::from(DELIVERY_LOG)),
                    ticket_lines: Vec::new(),
                }),
                Err(error) => Err(FailedRun {
                    error,
                    pr_url: Some(SPEC_PR.to_string()),
                    log: Some(PathBuf::from(DELIVERY_LOG)),
                    interrupted: false,
                    ticket_lines: Vec::new(),
                }),
            }
        }

        fn interrupted(&mut self) -> bool {
            if self
                .script
                .interrupted_after
                .is_some_and(|after| self.not_interrupted >= after)
            {
                return true;
            }
            self.not_interrupted += 1;
            false
        }

        fn step(&mut self, line: String) {
            self.did.push(Did::Line(line));
        }
    }

    /// Open Ticket `number`, blocked by `blockers`.
    fn open(number: u64, blockers: &[u64]) -> Node {
        Node {
            number,
            is_open: true,
            labels: Vec::new(),
            blockers: blockers.to_vec(),
        }
    }

    /// Open Ticket `number`, labelled `label`.
    fn labelled(number: u64, label: &'static str) -> Node {
        Node {
            labels: vec![label],
            ..open(number, &[])
        }
    }

    /// Closed Ticket `number`, blocked by none.
    fn closed(number: u64) -> Node {
        Node {
            is_open: false,
            ..open(number, &[])
        }
    }

    fn reached() -> Ended {
        Ended::Reached {
            pr_url: None,
            base_fix: None,
        }
    }

    fn failed(cause: &str) -> Ended {
        Ended::Failed {
            cause: cause.to_string(),
            log: Some("/logs/session.jsonl".to_string()),
        }
    }

    /// Land the Spec's Tickets, as `outside` has them, up to `parallel` at
    /// once.
    fn land(outside: &mut Scripted, parallel: usize) -> (Vec<String>, Result<String>) {
        let tickets = outside.graph();
        land_tickets(outside, tickets, NonZeroUsize::new(parallel).unwrap())
    }

    /// Take Spec #20, its Tickets as `outside` has them, up to `parallel`
    /// at once, from its Spec branch `issue-20` to a Spec PR into `main`.
    fn spec_run(outside: &mut Scripted, parallel: usize) -> Result<Reached, FailedRun> {
        let spec = IssueUrl::parse("https://github.com/acme/widgets/issues/20").unwrap();
        let tickets = outside.graph();
        let parallel = NonZeroUsize::new(parallel).unwrap();
        run_through(outside, &spec, "main", "issue-20", tickets, parallel)
    }

    /// The Reached of `ended`, which must be one.
    fn reached_of(ended: Result<Reached, FailedRun>) -> Reached {
        match ended {
            Ok(reached) => reached,
            Err(failed) => panic!("failed: {:#}", failed.error),
        }
    }

    /// The Failed spec run of `ended`, which must be one.
    fn failed_of(ended: Result<Reached, FailedRun>) -> FailedRun {
        match ended {
            Ok(reached) => panic!("reached {}", reached.pr_url),
            Err(failed) => failed,
        }
    }

    /// The Spec review's Opening, as the Delivery of Spec #20 starts it.
    fn spec_review_opening() -> Did {
        let spec = IssueUrl::parse("https://github.com/acme/widgets/issues/20").unwrap();
        Did::Deliver {
            kind: SPEC_REVIEW.to_string(),
            prompt: prompt::spec_review(&spec, "main", "issue-20", SPEC_PR),
            catch_up: true,
        }
    }

    /// Spec PR calls and the Delivery's steps only.
    fn after_the_loop(did: &Did) -> bool {
        matches!(
            did,
            Did::Landed(_) | Did::Show(_) | Did::PutBack(_) | Did::Deliver { .. } | Did::Ready
        )
    }

    /// The error of `result`, which must be one.
    fn error(result: Result<String>) -> String {
        format!("{:#}", result.expect_err("did not fail"))
    }

    /// The Tickets checklist of `lines`.
    fn checklist(lines: &[&str]) -> String {
        let mut list = "## Tickets\n\n".to_string();
        for line in lines {
            list += &format!("- {line}\n");
        }
        list
    }

    fn line(line: &str) -> Did {
        Did::Line(line.to_string())
    }

    /// Starts and endings only.
    fn runs(did: &Did) -> bool {
        matches!(did, Did::Start(_) | Did::Ended(_))
    }

    /// Spec PR calls only.
    fn spec_pr_calls(did: &Did) -> bool {
        matches!(did, Did::Show(_) | Did::Landed(_))
    }

    /// Progress lines only.
    fn progress_lines(did: &Did) -> bool {
        matches!(did, Did::Line(_))
    }

    #[test]
    fn it_pushes_then_lands_each_ticket_telling_the_spec_pr_the_checklist_read_after_each() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[21])],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        let done = checklist(&["[x] #21 landed", "[x] #22 landed"]);
        assert_eq!(landed.unwrap(), done);
        assert_eq!(lines, ["#21 landed", "#22 landed"]);
        assert_eq!(
            outside.did,
            [
                Did::Push,
                line("starting #21"),
                Did::Start(21),
                Did::Show(checklist(&["[ ] #21 running", "[ ] #22 blocked by #21"])),
                Did::Ended(21),
                Did::ReadTickets,
                Did::Landed(checklist(&["[x] #21 landed", "[ ] #22 not started"])),
                line("starting #22"),
                Did::Start(22),
                Did::Show(checklist(&["[x] #21 landed", "[ ] #22 running"])),
                Did::Ended(22),
                Did::ReadTickets,
                Did::Landed(done),
            ]
        );
    }

    #[test]
    fn a_push_that_fails_starts_nothing_and_returns_no_lines() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[])],
            push_fails: true,
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "git push origin issue-20 failed");
        assert!(lines.is_empty());
        assert_eq!(outside.did, [Did::Push]);
    }

    #[test]
    fn up_to_parallel_start_lowest_first_and_the_ticket_blocked_by_two_waits_for_both() {
        let mut outside = Scripted::new(Script {
            tickets: vec![
                open(21, &[]),
                open(22, &[]),
                open(23, &[21, 22]),
                open(24, &[]),
            ],
            endings: VecDeque::from([(22, Ok(reached()))]),
            ..Script::default()
        });

        let (_, landed) = land(&mut outside, 2);

        landed.unwrap();
        assert_eq!(
            outside.did_only(runs),
            [
                Did::Start(21),
                Did::Start(22),
                Did::Ended(22),
                Did::Start(24),
                Did::Ended(21),
                Did::Start(23),
                Did::Ended(24),
                Did::Ended(23),
            ]
        );
    }

    #[test]
    fn closed_tickets_are_never_started() {
        let mut outside = Scripted::new(Script {
            tickets: vec![closed(21), open(22, &[21])],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        landed.unwrap();
        assert_eq!(lines, ["#22 landed"]);
        assert_eq!(outside.did_only(runs), [Did::Start(22), Did::Ended(22)]);
    }

    #[test]
    fn a_failed_ticket_is_not_started_again_nor_what_it_blocks_while_an_independent_one_lands() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[21]), open(23, &[])],
            endings: VecDeque::from([(21, Ok(failed("claude exited 1")))]),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 1);

        assert_eq!(error(landed), "Tickets not done: #21, #22");
        let expected = [
            "#21 failed: claude exited 1 (session log: /logs/session.jsonl)",
            "#22 blocked by #21",
            "#23 landed",
        ];
        assert_eq!(lines, expected);
        assert_eq!(
            outside.did_only(runs),
            [
                Did::Start(21),
                Did::Ended(21),
                Did::Start(23),
                Did::Ended(23)
            ]
        );
        assert!(outside.did.ends_with(&expected.map(line)));
    }

    #[test]
    fn an_unready_ticket_and_what_it_blocks_are_not_started_and_are_named_in_the_lines() {
        let mut outside = Scripted::new(Script {
            tickets: vec![labelled(21, "needs-info"), open(22, &[21]), open(23, &[])],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "Tickets not done: #21, #22");
        assert_eq!(
            lines,
            [
                "#21 unready: labelled needs-info",
                "#22 blocked by #21",
                "#23 landed"
            ]
        );
        assert_eq!(outside.did_only(runs), [Did::Start(23), Did::Ended(23)]);
    }

    #[test]
    fn an_open_outside_blocker_holds_a_ticket_back_and_a_closed_one_does_not() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[90]), open(22, &[91])],
            open_outside: vec![90],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "Tickets not done: #21");
        assert_eq!(
            lines,
            ["#21 blocked by #90 (outside the Spec)", "#22 landed"]
        );
        assert_eq!(outside.did_only(runs), [Did::Start(22), Did::Ended(22)]);
    }

    #[test]
    fn tickets_in_a_cycle_are_not_started_and_their_lines_name_the_cycle() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[22]), open(22, &[21]), open(23, &[])],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "Tickets not done: #21, #22");
        assert_eq!(
            lines,
            [
                "#21 in a cycle: #21 blocked by #22 blocked by #21",
                "#22 in a cycle: #22 blocked by #21 blocked by #22",
                "#23 landed",
            ]
        );
        assert_eq!(outside.did_only(runs), [Did::Start(23), Did::Ended(23)]);
    }

    #[test]
    fn a_reread_that_removes_an_unready_label_lets_that_ticket_start_once_another_ends() {
        let mut outside = Scripted::new(Script {
            tickets: vec![labelled(21, "needs-info"), open(22, &[])],
            reads: VecDeque::from([Read::Changes(|tickets| tickets[0].labels.clear())]),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        landed.unwrap();
        assert_eq!(lines, ["#21 landed", "#22 landed"]);
        assert_eq!(
            outside.did_only(runs),
            [
                Did::Start(22),
                Did::Ended(22),
                Did::Start(21),
                Did::Ended(21)
            ]
        );
    }

    #[test]
    fn a_reread_that_fails_is_the_error_once_the_running_tickets_end_and_starts_nothing_more() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[]), open(23, &[])],
            reads: VecDeque::from([Read::Fails("gh: GitHub stopped answering")]),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 2);

        assert_eq!(error(landed), "gh: GitHub stopped answering");
        assert_eq!(lines, ["#21 landed", "#22 landed", "#23 not started"]);
        assert_eq!(
            outside.did_only(runs),
            [
                Did::Start(21),
                Did::Start(22),
                Did::Ended(21),
                Did::Ended(22)
            ]
        );
        assert_eq!(
            outside.did_only(progress_lines),
            [line("starting #21"), line("starting #22")]
        );
    }

    #[test]
    fn a_ticket_that_could_not_be_started_is_the_error_and_nothing_more_starts() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[]), open(23, &[])],
            start_fails: vec![22],
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "could not start #22's Run");
        assert_eq!(lines, ["#21 landed", "#22 not started", "#23 not started"]);
        assert_eq!(
            outside.did_only(runs),
            [Did::Start(21), Did::Start(22), Did::Ended(21)]
        );
        assert_eq!(
            outside.did_only(progress_lines),
            [line("starting #21"), line("starting #22")]
        );
    }

    #[test]
    fn a_run_that_could_not_be_waited_on_ends_failed_with_its_error_as_cause_and_the_loops_error() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[])],
            endings: VecDeque::from([(21, Err(anyhow!("could not wait for #21's Run")))]),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 1);

        assert_eq!(error(landed), "could not wait for #21's Run");
        assert_eq!(
            lines,
            [
                "#21 failed: could not wait for #21's Run",
                "#22 not started"
            ]
        );
        assert_eq!(outside.did_only(runs), [Did::Start(21), Did::Ended(21)]);
    }

    #[test]
    fn the_first_error_wins_over_later_ones() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[]), open(23, &[])],
            endings: VecDeque::from([
                (21, Err(anyhow!("could not wait for #21's Run"))),
                (22, Ok(reached())),
                (23, Err(anyhow!("could not wait for #23's Run"))),
            ]),
            reads: VecDeque::from([Read::Graph, Read::Fails("gh: GitHub stopped answering")]),
            landed_fails: true,
            ..Script::default()
        });

        let (_, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "could not wait for #21's Run");
        // After #22 lands, its re-read fails and so does opening the Spec PR.
        assert_eq!(
            outside.did_only(|did| matches!(did, Did::Ended(_) | Did::Landed(_))),
            [
                Did::Ended(21),
                Did::Ended(22),
                Did::Landed(checklist(&[
                    "[ ] #21 failed: could not wait for #21's Run",
                    "[ ] #22 landed, but is still open",
                    "[ ] #23 running"
                ])),
                Did::Ended(23),
            ]
        );
    }

    #[test]
    fn the_checklist_is_shown_after_each_round_that_started_a_ticket_and_not_after_one_that_started_none()
     {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[]), open(23, &[21, 22])],
            endings: VecDeque::from([(22, Ok(failed("claude exited 1")))]),
            ..Script::default()
        });

        let (_, landed) = land(&mut outside, 2);

        assert_eq!(error(landed), "Tickets not done: #22, #23");
        assert_eq!(
            outside.did_only(spec_pr_calls),
            [
                Did::Show(checklist(&[
                    "[ ] #21 running",
                    "[ ] #22 running",
                    "[ ] #23 blocked by #21, #22"
                ])),
                Did::Show(checklist(&[
                    "[ ] #21 running",
                    "[ ] #22 failed: claude exited 1",
                    "[ ] #23 blocked by #21, #22"
                ])),
                Did::Landed(checklist(&[
                    "[x] #21 landed",
                    "[ ] #22 failed: claude exited 1",
                    "[ ] #23 blocked by #22"
                ])),
            ]
        );
    }

    #[test]
    fn a_ticket_that_failed_or_was_interrupted_only_shows_the_checklist() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[])],
            endings: VecDeque::from([
                (21, Ok(failed("claude exited 1"))),
                (22, Ok(Ended::Interrupted)),
            ]),
            ..Script::default()
        });

        let (_, landed) = land(&mut outside, 2);

        assert_eq!(error(landed), "Tickets not done: #21, #22");
        assert_eq!(
            outside.did_only(spec_pr_calls),
            [
                Did::Show(checklist(&["[ ] #21 running", "[ ] #22 running"])),
                Did::Show(checklist(&[
                    "[ ] #21 failed: claude exited 1",
                    "[ ] #22 running"
                ])),
                Did::Show(checklist(&[
                    "[ ] #21 failed: claude exited 1",
                    "[ ] #22 interrupted"
                ])),
            ]
        );
    }

    #[test]
    fn a_spec_pr_that_could_not_be_opened_is_the_error_and_nothing_more_starts() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[])],
            landed_fails: true,
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 1);

        assert_eq!(error(landed), "could not open the Spec PR");
        assert_eq!(lines, ["#21 landed", "#22 not started"]);
        assert_eq!(outside.did_only(runs), [Did::Start(21), Did::Ended(21)]);
    }

    #[test]
    fn an_interrupt_before_any_ticket_starts_starts_none_and_each_is_not_started() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[])],
            interrupted_after: Some(0),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 3);

        assert_eq!(error(landed), "interrupted");
        assert_eq!(lines, ["#21 not started", "#22 not started"]);
        assert_eq!(outside.did, [Did::Push]);
    }

    #[test]
    fn an_interrupt_while_tickets_run_lets_them_end_and_starts_no_more() {
        // Found not interrupted before #21, before #22 and once two run.
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[]), open(23, &[])],
            interrupted_after: Some(3),
            ..Script::default()
        });

        let (lines, landed) = land(&mut outside, 2);

        assert_eq!(error(landed), "interrupted");
        assert_eq!(lines, ["#21 landed", "#22 landed", "#23 not started"]);
        assert_eq!(
            outside.did_only(runs),
            [
                Did::Start(21),
                Did::Start(22),
                Did::Ended(21),
                Did::Ended(22)
            ]
        );
        assert_eq!(
            outside.did_only(progress_lines),
            [line("starting #21"), line("starting #22")]
        );
    }

    #[test]
    fn once_every_ticket_lands_the_spec_review_opens_the_delivery_and_the_checklist_goes_back_before_ready()
     {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[])],
            ..Script::default()
        });

        let reached = reached_of(spec_run(&mut outside, 3));

        let done = checklist(&["[x] #21 landed"]);
        assert_eq!(
            outside.did_only(after_the_loop),
            [
                Did::Show(checklist(&["[ ] #21 running"])),
                Did::Landed(done.clone()),
                Did::Landed(done.clone()),
                spec_review_opening(),
                Did::PutBack(done),
                Did::Ready,
            ]
        );
        assert_eq!(reached.pr_url, SPEC_PR);
        assert_eq!(reached.log, Some(PathBuf::from(DELIVERY_LOG)));
        assert_eq!(reached.ticket_lines, ["#21 landed"]);
    }

    #[test]
    fn a_failed_landing_means_no_delivery_and_carries_the_spec_prs_url_and_the_ticket_lines() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[]), open(22, &[])],
            endings: VecDeque::from([(21, Ok(failed("claude exited 1")))]),
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 1));

        assert_eq!(format!("{:#}", failed.error), "Tickets not done: #21");
        assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
        assert_eq!(failed.log, None);
        assert!(!failed.interrupted);
        assert_eq!(
            failed.ticket_lines,
            [
                "#21 failed: claude exited 1 (session log: /logs/session.jsonl)",
                "#22 landed"
            ]
        );
        assert!(
            !outside
                .did
                .iter()
                .any(|did| matches!(did, Did::Deliver { .. }))
        );
    }

    #[test]
    fn a_failed_landing_with_no_spec_pr_open_carries_no_pr_url() {
        let mut outside = Scripted::new(Script {
            tickets: vec![labelled(21, "needs-info")],
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        assert_eq!(failed.pr_url, None);
        assert_eq!(failed.ticket_lines, ["#21 unready: labelled needs-info"]);
    }

    #[test]
    fn a_failed_landing_with_an_interrupt_requested_ends_as_interrupted() {
        // Found not interrupted before #21, the only time the loop asks once
        // its start failed; the interrupt comes as the landing fails.
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[])],
            spec_pr_open: true,
            start_fails: vec![21],
            interrupted_after: Some(1),
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        assert!(failed.interrupted);
        assert_eq!(format!("{:#}", failed.error), "interrupted");
        assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
        assert_eq!(failed.ticket_lines, ["#21 not started"]);
    }

    #[test]
    fn a_spec_pr_that_cannot_be_opened_after_the_last_landing_means_no_delivery() {
        let mut outside = Scripted::new(Script {
            tickets: vec![closed(21)],
            landed_fails: true,
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        assert_eq!(format!("{:#}", failed.error), "could not open the Spec PR");
        assert_eq!(failed.pr_url, None);
        assert_eq!(
            outside.did_only(after_the_loop),
            [Did::Landed(checklist(&["[x] #21 done"]))]
        );
    }

    #[test]
    fn a_failed_delivery_shows_the_checklist_again_and_keeps_its_pr_url_log_and_the_ticket_lines() {
        let mut outside = Scripted::new(Script {
            tickets: vec![open(21, &[])],
            delivery_fails: true,
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        let done = checklist(&["[x] #21 landed"]);
        assert_eq!(
            format!("{:#}", failed.error),
            "checks failed on the Spec PR"
        );
        assert_eq!(
            outside.did_only(after_the_loop),
            [
                Did::Show(checklist(&["[ ] #21 running"])),
                Did::Landed(done.clone()),
                Did::Landed(done.clone()),
                spec_review_opening(),
                Did::PutBack(done.clone()),
                Did::Ready,
                Did::Show(done),
            ]
        );
        assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
        assert_eq!(failed.log, Some(PathBuf::from(DELIVERY_LOG)));
        assert_eq!(failed.ticket_lines, ["#21 landed"]);
    }

    #[test]
    fn a_put_back_that_fails_fails_the_delivery_before_ready() {
        let mut outside = Scripted::new(Script {
            tickets: vec![closed(21)],
            put_back_fails: true,
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        let done = checklist(&["[x] #21 done"]);
        assert_eq!(
            format!("{:#}", failed.error),
            "could not put the Tickets checklist back"
        );
        assert_eq!(
            outside.did_only(after_the_loop),
            [
                Did::Landed(done.clone()),
                spec_review_opening(),
                Did::PutBack(done.clone()),
                Did::Show(done),
            ]
        );
    }

    #[test]
    fn a_delivery_that_fails_in_the_spec_review_shows_the_checklist_again_without_a_put_back() {
        let mut outside = Scripted::new(Script {
            tickets: vec![closed(21)],
            review_fails: true,
            ..Script::default()
        });

        let failed = failed_of(spec_run(&mut outside, 3));

        let done = checklist(&["[x] #21 done"]);
        assert_eq!(
            format!("{:#}", failed.error),
            "the spec-review session failed"
        );
        assert_eq!(
            outside.did_only(after_the_loop),
            [
                Did::Landed(done.clone()),
                spec_review_opening(),
                Did::Show(done)
            ]
        );
    }
}
