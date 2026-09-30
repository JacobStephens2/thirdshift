//! A Spec run: each of a Spec's Tickets, in dependency order, taken by a
//! Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec PR from the Spec branch into the Base branch.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

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

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base`,
/// ready for review. The Spec branch is pushed before any Ticket starts, and
/// the Tickets run one at a time, the graph read again after each. A Ticket
/// that fails stops only the Tickets it blocks; with any Ticket not done,
/// this is a Failed spec run. The worktree is cleaned up when this returns.
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

/// How a Ticket's Run in this Spec run ended.
enum Ran {
    /// Its PR, as the Run printed it.
    Landed(Option<String>),
    /// Why, and its session log, as the Run reported them.
    Failed { cause: String, log: Option<String> },
}

/// Push the Spec branch, then run each ready Ticket, lowest number first,
/// until none is ready. Fails, after a line on each Ticket that landed or is
/// not done, if any Ticket is not done.
fn land_tickets(spec: &IssueUrl, mut tickets: Vec<Ticket>, worktree: &Worktree) -> Result<()> {
    worktree.push()?;
    let mut ran = BTreeMap::new();
    while let Some(ticket) = next_ready(&tickets, &ran) {
        if interrupt::requested() {
            bail!("interrupted");
        }
        let outcome = run_ticket(spec, ticket, worktree.branch())?;
        ran.insert(ticket, outcome);
        tickets = github::tickets(spec)?;
    }
    let not_done: Vec<String> = tickets
        .iter()
        .filter(|ticket| ticket.is_open)
        .map(|ticket| format!("#{}", ticket.number))
        .collect();
    if not_done.is_empty() {
        return Ok(());
    }
    report(&tickets, &ran);
    bail!("Tickets not done: {}", not_done.join(", "));
}

/// The lowest-numbered Ticket that is open, not an Unready Ticket, has no
/// sub-issues, has every blocker closed and hasn't `ran` yet.
fn next_ready(tickets: &[Ticket], ran: &BTreeMap<u64, Ran>) -> Option<u64> {
    tickets
        .iter()
        .filter(|ticket| {
            ticket.is_open
                && !ticket.has_sub_issues
                && ticket.open_blockers.is_empty()
                && !ran.contains_key(&ticket.number)
                && unready_label(ticket).is_none()
        })
        .map(|ticket| ticket.number)
        .min()
}

/// The first of the Unready Ticket labels `ticket` has, if any.
fn unready_label(ticket: &Ticket) -> Option<&str> {
    UNREADY_LABELS
        .into_iter()
        .find(|label| ticket.labels.iter().any(|name| name == label))
}

/// A line on each Ticket that landed in this Spec run, with its PR, and on
/// each Ticket not done, with why, lowest number first.
fn report(tickets: &[Ticket], ran: &BTreeMap<u64, Ran>) {
    let mut sorted: Vec<&Ticket> = tickets.iter().collect();
    sorted.sort_by_key(|ticket| ticket.number);
    for ticket in sorted {
        let number = ticket.number;
        let landed = match ran.get(&number) {
            Some(Ran::Landed(Some(pr_url))) => Some(format!("landed with {pr_url}")),
            Some(Ran::Landed(None)) => Some("landed".to_string()),
            _ => None,
        };
        let line = if !ticket.is_open {
            match landed {
                Some(landed) => landed,
                None => continue,
            }
        } else if let Some(Ran::Failed { cause, log }) = ran.get(&number) {
            match log {
                Some(log) => format!("failed: {cause} (session log: {log})"),
                None => format!("failed: {cause}"),
            }
        } else if let Some(landed) = landed {
            format!("{landed}, but is still open")
        } else if let Some(label) = unready_label(ticket) {
            format!("unready: labelled {label}")
        } else if ticket.has_sub_issues {
            "unready: has sub-issues".to_string()
        } else if let Some(cycle) = cycle_through(number, tickets) {
            let cycle: Vec<String> = cycle.iter().map(|number| format!("#{number}")).collect();
            format!("in a cycle: {}", cycle.join(" blocked by "))
        } else {
            let blockers: Vec<String> = ticket
                .open_blockers
                .iter()
                .map(|blocker| {
                    if tickets.iter().any(|ticket| ticket.number == *blocker) {
                        format!("#{blocker}")
                    } else {
                        format!("#{blocker} (outside the Spec)")
                    }
                })
                .collect();
            format!("blocked by {}", blockers.join(", "))
        };
        progress::step(format_args!("#{number} {line}"));
    }
}

/// The shortest cycle of "blocked by" links among `tickets` from Ticket
/// `start` back to itself, `start` first and last, if there is one.
fn cycle_through(start: u64, tickets: &[Ticket]) -> Option<Vec<u64>> {
    let blockers = |number: u64| {
        tickets
            .iter()
            .find(|ticket| ticket.number == number)
            .map_or(&[][..], |ticket| &ticket.open_blockers[..])
    };
    let mut reached_from = HashMap::new();
    let mut queue = VecDeque::from([start]);
    while let Some(number) = queue.pop_front() {
        for &blocker in blockers(number) {
            if blocker == start {
                let mut cycle = vec![start];
                let mut at = number;
                while at != start {
                    cycle.push(at);
                    at = reached_from[&at];
                }
                cycle.push(start);
                cycle.reverse();
                return Some(cycle);
            }
            if let Entry::Vacant(entry) = reached_from.entry(blocker) {
                entry.insert(number);
                queue.push_back(blocker);
            }
        }
    }
    None
}

/// Run Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory, relaying its stderr with a
/// `#<number>: ` prefix. It landed if it exits 0, and failed otherwise, with
/// the cause and session log it ended on.
fn run_ticket(spec: &IssueUrl, number: u64, spec_branch: &str) -> Result<Ran> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let mut child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    // A failed Run ends on its error, then its session log if it has one.
    let mut last_lines: [Option<String>; 2] = [None, None];
    for line in BufReader::new(stderr).lines() {
        let line = line.with_context(|| format!("could not read the Run for #{number}"))?;
        let line = line
            .strip_prefix("thirdshift: ")
            .unwrap_or(&line)
            .to_string();
        progress::step(format_args!("#{number}: {line}"));
        last_lines = [last_lines[1].take(), Some(line)];
    }
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .context("no stdout from the Run")?
        .read_to_string(&mut stdout)
        .with_context(|| format!("could not read the Run for #{number}"))?;
    let status = child
        .wait()
        .with_context(|| format!("could not wait for the Run for #{number}"))?;
    if status.success() {
        progress::step(format_args!("#{number} landed"));
        return Ok(Ran::Landed(stdout.lines().last().map(String::from)));
    }
    progress::step(format_args!("#{number} failed"));
    let [before, last] = last_lines;
    let (cause, log) = match last {
        Some(last) => match last.strip_prefix("session log: ") {
            Some(log) => (before, Some(log.to_string())),
            None => (Some(last), None),
        },
        None => (None, None),
    };
    Ok(Ran::Failed {
        cause: cause.unwrap_or_else(|| format!("the Run {status}")),
        log,
    })
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
