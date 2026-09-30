//! A Spec run: each of a Spec's Tickets, in dependency order, taken by a
//! Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec PR from the Spec branch into the Base branch.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::args;
use crate::failed_run::FailedRun;
use crate::github::{self, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::run::{Goal, Reached};
use crate::worktree::Worktree;

/// How often to check whether a Ticket's Run has ended, or been interrupted.
const POLL: Duration = Duration::from_millis(100);

/// The triage labels that make an open Ticket an Unready Ticket.
const UNREADY_LABELS: [&str; 4] = ["ready-for-human", "needs-info", "wontfix", "needs-triage"];

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base`,
/// ready for review. The Spec branch is pushed before any Ticket starts, and
/// the Tickets run one at a time, the graph read again after each. A Ticket
/// that fails ends the Spec run. An interrupt ends it too, once the running
/// Ticket's Run has ended, starting nothing more and leaving the Spec PR
/// not ready. The worktree is cleaned up when this returns.
pub fn run(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: Worktree,
    base: &str,
) -> Result<Reached, FailedRun> {
    let pr_url = land_tickets(spec, tickets, &worktree)
        .and_then(|()| open_spec_pr(spec, worktree.branch(), base))?;
    Ok(Reached {
        pr_url,
        goal: Goal::ReadyForReview,
        log: None,
    })
}

/// Push the Spec branch, then run each ready Ticket, lowest number first,
/// until none is ready. Fails if a Ticket fails, or if any is left open.
fn land_tickets(spec: &IssueUrl, mut tickets: Vec<Ticket>, worktree: &Worktree) -> Result<()> {
    worktree.push()?;
    let mut started = HashSet::new();
    while let Some(ticket) = next_ready(&tickets, &started) {
        if interrupt::requested() {
            bail!("interrupted");
        }
        started.insert(ticket);
        run_ticket(spec, ticket, worktree.branch())?;
        tickets = github::tickets(spec)?;
    }
    let open: Vec<String> = tickets
        .iter()
        .filter(|ticket| ticket.is_open)
        .map(|ticket| format!("#{}", ticket.number))
        .collect();
    if !open.is_empty() {
        bail!("no Ticket is ready, and {} still open", open.join(", "));
    }
    Ok(())
}

/// The lowest-numbered Ticket that is open, not an Unready Ticket, has no
/// sub-issues, has every blocker closed and hasn't been `started` yet.
fn next_ready(tickets: &[Ticket], started: &HashSet<u64>) -> Option<u64> {
    tickets
        .iter()
        .filter(|ticket| {
            ticket.is_open
                && !ticket.has_sub_issues
                && ticket.open_blockers.is_empty()
                && !started.contains(&ticket.number)
                && !ticket
                    .labels
                    .iter()
                    .any(|label| UNREADY_LABELS.contains(&label.as_str()))
        })
        .map(|ticket| ticket.number)
        .min()
}

/// Run Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory, relaying its stderr with a
/// `#<number>: ` prefix. An interrupt is passed on to the child, which is
/// waited for as it goes down its Failed run path. Fails unless it exits 0.
fn run_ticket(spec: &IssueUrl, number: u64, spec_branch: &str) -> Result<()> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let mut child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
    // Relay on its own thread, so this one can watch for an interrupt.
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    let relay = thread::spawn(move || -> std::io::Result<()> {
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            let line = line.strip_prefix("thirdshift: ").unwrap_or(&line);
            progress::step(format_args!("#{number}: {line}"));
        }
        Ok(())
    });
    let mut passed_on = false;
    let status = loop {
        // The child shares the process group, so a Ctrl-C or a closed
        // terminal reaches it too, but a signal sent to this process alone
        // doesn't. A second one is harmless: it only records the interrupt.
        if !passed_on && interrupt::requested() {
            // SAFETY: kill has no memory-safety preconditions, and the child
            // is not yet reaped, so its pid is still its own.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            passed_on = true;
        }
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("could not wait for the Run for #{number}"))?
        {
            break status;
        }
        thread::sleep(POLL);
    };
    relay
        .join()
        .map_err(|_| anyhow!("the relay of the Run for #{number} panicked"))?
        .with_context(|| format!("could not read the Run for #{number}"))?;
    if !status.success() {
        if interrupt::requested() {
            progress::step(format_args!("#{number} interrupted"));
            bail!("interrupted");
        }
        bail!("#{number} failed");
    }
    progress::step(format_args!("#{number} landed"));
    Ok(())
}

/// The Spec PR from `spec_branch` into `base`, ready for review: the open
/// one if there is one, else a new one titled from the Spec that closes it.
fn open_spec_pr(spec: &IssueUrl, spec_branch: &str, base: &str) -> Result<String> {
    if interrupt::requested() {
        bail!("interrupted");
    }
    if let Some(pr) = github::pull_request_for(spec, spec_branch)?.filter(|pr| pr.is_open()) {
        if pr.is_draft {
            github::mark_ready(spec, spec_branch)?;
        }
        return Ok(pr.url);
    }
    progress::step(format_args!("opening the Spec PR into {base}"));
    let title = github::issue_title(spec)?;
    let body = format!(
        "The work on Spec #{number}, gathered from its Tickets on {spec_branch}.\n\n\
         Closes #{number}\n",
        number = spec.number
    );
    github::create_pr(spec, spec_branch, base, &title, &body)
}
