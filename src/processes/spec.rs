//! The Built-in Spec process: its file owns the sequence while the Ticket
//! loop and Delivery keep their existing lifecycle and failure accounting.

use std::num::NonZeroUsize;

use anyhow::{Context, Result, ensure};

use super::{DraftKind, WorktreeKind, builtin};
use crate::delivery::{Opening, OpeningSession};
use crate::failed_run::FailedRun;
use crate::github::Ticket;
use crate::issue::IssueUrl;
use crate::prompt;
use crate::run::Reached;
use crate::spec_run::{Outside, account_tickets};
use builtin::Step;

pub(crate) struct Sequence {
    pub process: String,
    pub delivery: Vec<Step>,
}

pub(crate) fn sequence() -> Result<Sequence> {
    let mut steps = builtin::steps("spec-run", WorktreeKind::SpecBranch)?.into_iter();
    let land = steps.next().context("spec-run needs land-tickets first")?;
    let DraftKind::LandTickets { line } = land.kind else {
        anyhow::bail!("spec-run needs land-tickets first");
    };
    let process = land.child_process(line).0.to_string();
    Ok(Sequence {
        process,
        delivery: steps
            .map(|step| {
                ensure!(
                    !matches!(step.kind, DraftKind::Implement { .. }),
                    "spec-run cannot implement a Ticket"
                );
                builtin::delivery_step(step)
            })
            .collect::<Result<_>>()?,
    })
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
pub(crate) fn run_through(
    outside: &mut impl Outside,
    spec: &IssueUrl,
    base: &str,
    branch: &str,
    tickets: Vec<Ticket>,
    parallel: NonZeroUsize,
    steps: Vec<Step>,
) -> Result<Reached, FailedRun> {
    let landed = account_tickets(outside, spec, tickets, parallel)?;
    match review_and_deliver(
        outside,
        spec,
        base,
        branch,
        &landed.checklist,
        &landed.pr_url,
        steps,
    ) {
        Ok(reached) => Ok(Reached {
            ticket_lines: landed.lines,
            ..reached
        }),
        Err(mut failed) => {
            landed.record_failure(&mut failed);
            Err(failed)
        }
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
    steps: Vec<Step>,
) -> Result<Reached, FailedRun> {
    if outside.interrupted() {
        return Err(FailedRun {
            interrupted: true,
            pr_url: Some(spec_pr_url.to_string()),
            ..FailedRun::from(anyhow::anyhow!("interrupted"))
        });
    }
    let opening = Opening {
        session: OpeningSession::SpecReview,
        prompt: prompt::spec_review(spec, base, branch, spec_pr_url),
    };
    // The Spec review may have rewritten the body without the checklist.
    let delivered = outside.deliver(opening, checklist, steps);
    if delivered.is_err() {
        // The Delivery's own failure is the cause; this only shows progress.
        let _ = outside.show(checklist);
    }
    delivered
}
