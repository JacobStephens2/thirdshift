//! A Spec run: each of a Spec's Tickets, in dependency order and several at
//! once, taken by a Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec PR from the Spec branch into the Base branch.

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::num::NonZeroUsize;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};

use anyhow::{Context, Result, bail};

use crate::args;
use crate::failed_run::FailedRun;
use crate::github::{self, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::run::{Goal, Reached};
use crate::worktree::Worktree;

/// The triage labels that make an open Ticket an Unready Ticket.
const UNREADY_LABELS: [&str; 4] = ["ready-for-human", "needs-info", "wontfix", "needs-triage"];

/// How many Tickets a Spec run runs at once.
pub struct Parallel {
    pub tickets: NonZeroUsize,
    /// Whether the command asked for it with `parallel <n>`, rather than the
    /// User config or the default deciding.
    pub asked: bool,
}

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base`,
/// ready for review. The Spec branch is pushed before any Ticket starts, and
/// up to `parallel` Tickets run at once, the graph read again whenever one
/// ends. A Ticket that fails stops any more from starting, and the Spec run
/// fails once those running have ended. The worktree is cleaned up when this
/// returns.
pub fn run(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: Worktree,
    base: &str,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let pr_url = land_tickets(spec, tickets, &worktree, parallel)
        .and_then(|()| open_spec_pr(spec, worktree.branch(), base))?;
    Ok(Reached {
        pr_url,
        goal: Goal::ReadyForReview,
        log: None,
    })
}

/// Push the Spec branch, then keep up to `parallel` Tickets running, each
/// time one ends starting ready ones, lowest number first, until none is
/// ready and none is running. Fails if a Ticket fails, or if any is left
/// open.
fn land_tickets(
    spec: &IssueUrl,
    mut tickets: Vec<Ticket>,
    worktree: &Worktree,
    parallel: NonZeroUsize,
) -> Result<()> {
    worktree.push()?;
    let (ended, endings) = mpsc::channel();
    let mut started = HashSet::new();
    let mut running = 0;
    let mut failure = None;
    loop {
        while failure.is_none() && !interrupt::requested() && running < parallel.get() {
            let Some(ticket) = next_ready(&tickets, &started) else {
                break;
            };
            started.insert(ticket);
            match start_ticket(spec, ticket, worktree.branch(), ended.clone()) {
                Ok(()) => running += 1,
                Err(error) => failure = Some(error),
            }
        }
        if running == 0 {
            break;
        }
        let result = endings
            .recv()
            .expect("a running Ticket's thread holds a sender");
        running -= 1;
        // Once one has failed, the rest are only waited for.
        match result.and_then(|()| github::tickets(spec)) {
            Ok(read) => tickets = read,
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    if let Some(error) = failure {
        return Err(error);
    }
    if interrupt::requested() {
        bail!("interrupted");
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

/// Start Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory, and a thread that relays
/// its stderr and sends how it ended on `ended`.
fn start_ticket(
    spec: &IssueUrl,
    number: u64,
    spec_branch: &str,
    ended: Sender<Result<()>>,
) -> Result<()> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
    std::thread::spawn(move || {
        let _ = ended.send(finish_ticket(number, child));
    });
    Ok(())
}

/// Relay the stderr of Ticket `number`'s Run `child` with a `#<number>: `
/// prefix until it exits. Fails unless it exits 0.
fn finish_ticket(number: u64, mut child: Child) -> Result<()> {
    let relayed = relay(number, &mut child);
    let status = child
        .wait()
        .with_context(|| format!("could not wait for the Run for #{number}"))?;
    relayed?;
    if !status.success() {
        bail!("#{number} failed");
    }
    progress::step(format_args!("#{number} landed"));
    Ok(())
}

fn relay(number: u64, child: &mut Child) -> Result<()> {
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    for line in BufReader::new(stderr).lines() {
        let line = line.with_context(|| format!("could not read the Run for #{number}"))?;
        let line = line.strip_prefix("thirdshift: ").unwrap_or(&line);
        progress::step(format_args!("#{number}: {line}"));
    }
    Ok(())
}

/// The Spec PR from `spec_branch` into `base`, ready for review: the open
/// one if there is one, else a new one titled from the Spec that closes it.
fn open_spec_pr(spec: &IssueUrl, spec_branch: &str, base: &str) -> Result<String> {
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
