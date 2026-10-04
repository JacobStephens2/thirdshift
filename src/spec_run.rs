//! A Spec run: each of a Spec's Tickets, in dependency order and several at
//! once, taken by a Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch, kept mergeable and green like a Run's PR, and Self-merged
//! when the Spec run was asked to merge.

mod spec_pr;
mod ticket_board;

use std::num::NonZeroUsize;
use std::sync::mpsc::{self, Sender};
use std::thread;

use anyhow::{Result, bail};

use crate::base_fix::BaseFixAsk;
use crate::child_run::{self, Ended, Handle, Kind};
use crate::delivery::{Delivery, Opening};
use crate::failed_run::FailedRun;
use crate::github::{self, Ticket};
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
/// into `delivery`'s Base branch that reaches its goal. The Spec branch is
/// pushed before any Ticket starts, and up to `parallel` Tickets run at once,
/// the graph read again whenever one ends. The Spec PR is opened as a draft
/// once the first Ticket lands, or turned back into a draft if it is already
/// open, and its Tickets checklist rewritten as Tickets start and end. A
/// Ticket that fails stops only the Tickets it blocks; with any Ticket not
/// done, this is a Failed spec run, which leaves the Spec PR, if there is
/// one, a draft. An interrupt ends it too, once the running Tickets' Runs
/// have ended, starting nothing more. Once every Ticket has landed, the
/// Spec PR is taken to its goal by `delivery`, opening with the Spec review
/// on the Spec branch caught up from origin, and the Tickets checklist put
/// back in the Spec PR's body before it is marked ready, and again if the
/// Delivery fails. The worktree is cleaned up when this returns, or kept by
/// the Failed run path if its work did not reach origin. However it ends, it
/// carries a line on each Ticket it landed or did not get done, for the Run
/// notification. If `delivery`'s Base fix may start one, so may each
/// Ticket's Run, into the Spec branch.
pub fn run(
    tickets: Vec<Ticket>,
    worktree: Worktree,
    delivery: Delivery,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let (spec, base) = (delivery.issue, delivery.base);
    let mut spec_pr = SpecPr::resume(spec, worktree.branch(), base)?;
    let (ticket_lines, landed) = land_tickets(
        spec,
        tickets,
        &worktree,
        parallel,
        &delivery.base_fix.ask_of_tickets(),
        &mut spec_pr,
    );
    let ended = match landed {
        Ok(checklist) => review_and_deliver(worktree, spec_pr, &checklist, delivery),
        Err(error) => Err(FailedRun {
            pr_url: spec_pr.url().map(str::to_string),
            ..FailedRun::from(error)
        }),
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

/// Once every Ticket has landed: tell `spec_pr` so, with `checklist`, which
/// opens it if it is not open, then take it to its goal by `delivery`,
/// opening with the Spec review, as [`run`] does.
fn review_and_deliver(
    worktree: Worktree,
    mut spec_pr: SpecPr,
    checklist: &str,
    delivery: Delivery,
) -> Result<Reached, FailedRun> {
    let (spec, base) = (delivery.issue, delivery.base);
    let spec_pr_url = spec_pr.landed(checklist)?;
    let opening = Opening {
        kind: SPEC_REVIEW,
        prompt: prompt::spec_review(spec, base, worktree.branch(), spec_pr_url),
        catch_up_from_origin: true,
    };
    // The Spec review may have rewritten the body without the checklist.
    let delivered = delivery.deliver(worktree, opening, || spec_pr.put_back(checklist));
    if delivered.is_err() {
        spec_pr.show(checklist);
    }
    delivered
}

/// Push the Spec branch, then keep up to `parallel` Tickets running, each
/// time one ends starting ready ones, lowest number first, until none is
/// ready and none is running, telling `spec_pr` the Tickets checklist as
/// Tickets start and end, and when a Ticket lands. Returns a line on each Ticket that landed or is
/// not done, none if the Spec branch could not be pushed, and the last
/// Tickets checklist if every Ticket is done: if not, it fails, after putting
/// those lines on stderr unless an interrupt or another error ended it first.
/// Each Ticket's Run may start a Base fix if `base_fix` allows one.
fn land_tickets(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: &Worktree,
    parallel: NonZeroUsize,
    base_fix: &BaseFixAsk,
    spec_pr: &mut SpecPr,
) -> (Vec<String>, Result<String>) {
    if let Err(error) = worktree.push() {
        return (Vec::new(), Err(error));
    }
    let mut board = TicketBoard::new(tickets, parallel);
    let landing = run_ready_tickets(spec, &mut board, worktree.branch(), base_fix, spec_pr);
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
            progress::step(line);
        }
        bail!("Tickets not done: {}", not_done.join(", "));
    });
    (lines, landed)
}

/// Keep the Tickets on `board` running as it offers them, until it offers
/// none and none is running, telling it as each starts and ends and as the
/// graph is read again, and telling `spec_pr` the Tickets checklist as they
/// start and end, and when one lands. An interrupt, or an
/// error other than a Ticket failing, stops any more from starting, and
/// fails this once those running have ended. Each Ticket's Run may start a
/// Base fix if `base_fix` allows one.
fn run_ready_tickets(
    spec: &IssueUrl,
    board: &mut TicketBoard,
    spec_branch: &str,
    base_fix: &BaseFixAsk,
    spec_pr: &mut SpecPr,
) -> Result<()> {
    let (ended, endings) = mpsc::channel();
    let mut error = None;
    loop {
        let mut started = false;
        loop {
            if error.is_some() || interrupt::requested() {
                board.stop();
            }
            let Some(ticket) = board.next_to_start() else {
                break;
            };
            match start_ticket(spec, ticket, spec_branch, base_fix, ended.clone()) {
                Ok(()) => {
                    board.started(ticket);
                    started = true;
                }
                Err(start_error) => error = Some(start_error),
            }
        }
        if started {
            spec_pr.show(&board.checklist());
        }
        if !board.any_running() {
            break;
        }
        let (ticket, result) = endings
            .recv()
            .expect("a running Ticket's thread holds a sender");
        // Once there is an error, the rest are only waited for, but each
        // still has its line in the checklist, from the last graph read.
        let ending = result.unwrap_or_else(|finish_error| {
            let cause = format!("{finish_error:#}");
            error.get_or_insert(finish_error);
            Ended::Failed { cause, log: None }
        });
        let landed = matches!(ending, Ended::Reached { .. });
        board.ended(ticket, ending);
        match github::tickets(spec) {
            Ok(reread) => board.reread(reread),
            Err(reread_error) => {
                error.get_or_insert(reread_error);
            }
        }
        let list = board.checklist();
        if landed {
            if let Err(open_error) = spec_pr.landed(&list) {
                error.get_or_insert(open_error);
            }
        } else {
            spec_pr.show(&list);
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    if interrupt::requested() {
        bail!("interrupted");
    }
    Ok(())
}

/// Start Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory that may start a Base fix if
/// `base_fix` allows one, and a thread that sends `number` and how it ended
/// on `ended`.
fn start_ticket(
    spec: &IssueUrl,
    number: u64,
    spec_branch: &str,
    base_fix: &BaseFixAsk,
    ended: Sender<(u64, Result<Ended>)>,
) -> Result<()> {
    progress::step(format_args!("starting #{number}"));
    let kind = Kind::Ticket {
        spec_branch: spec_branch.to_string(),
    };
    let child = child_run::start(&spec.sibling(number), kind, base_fix.clone())?;
    thread::spawn(move || {
        let _ = ended.send((number, finish_ticket(number, child)));
    });
    Ok(())
}

/// Wait for Ticket `number`'s Run `child`, relaying its stderr, and say how
/// it ended.
fn finish_ticket(number: u64, child: Handle) -> Result<Ended> {
    let ended = child.wait()?;
    progress::step(match ended {
        Ended::Reached { .. } => format!("#{number} landed"),
        Ended::Interrupted => format!("#{number} interrupted"),
        Ended::Failed { .. } => format!("#{number} failed"),
    });
    Ok(ended)
}
