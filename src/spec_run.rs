//! A Spec run: each of a Spec's Tickets, in dependency order and several at
//! once, taken by a Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch, kept mergeable and green like a Run's PR, and Self-merged
//! when the Spec run was asked to merge.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::args;
use crate::failed_run::{self, FailedRun};
use crate::github::{self, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::plugin::Plugin;
use crate::progress;
use crate::prompt;
use crate::run::{self, Goal, Reached};
use crate::session::{Logs, Sessions};
use crate::worktree::Worktree;

/// How often to check whether a Ticket's Run has ended, or been interrupted.
const POLL: Duration = Duration::from_millis(100);

/// The triage labels that make an open Ticket an Unready Ticket.
const UNREADY_LABELS: [&str; 4] = ["ready-for-human", "needs-info", "wontfix", "needs-triage"];

/// The Spec review session's kind, in its progress lines and log name.
const SPEC_REVIEW: &str = "spec-review";

/// How many Tickets a Spec run runs at once.
pub struct Parallel {
    pub tickets: NonZeroUsize,
    /// Whether the command asked for it with `parallel <n>`, rather than the
    /// User config or the default deciding.
    pub asked: bool,
}

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base` that
/// reaches `goal`. The Spec branch is pushed before any Ticket starts, and
/// up to `parallel` Tickets run at once, the graph read again whenever one
/// ends. A Ticket that fails stops only the Tickets it blocks; with any
/// Ticket not done, this is a Failed spec run. An interrupt ends it too, once
/// the running Tickets' Runs have ended, starting nothing more and leaving the Spec PR
/// not ready. Once every Ticket has landed, the Spec PR is opened as a draft
/// and the Spec review, logged in `logs`, reviews the Spec branch before the
/// Spec PR is marked ready. The Spec PR then goes through the same Repair
/// loop as a Run's PR, and for [`Goal::Merged`] the Self-merge. A failure
/// from the Spec review on goes through the Failed run path. The worktree is
/// cleaned up when this returns, or kept by the Failed run path if its work
/// did not reach origin. However it ends, it carries a line on each Ticket
/// it landed or did not get done, for the Run notification.
pub fn run(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: Worktree,
    base: &str,
    goal: Goal,
    logs: &Logs,
    parallel: NonZeroUsize,
) -> Result<Reached, FailedRun> {
    let (ticket_lines, landed) = land_tickets(spec, tickets, &worktree, parallel);
    let ended = landed
        .map_err(FailedRun::from)
        .and_then(|()| open_and_review(spec, worktree, base, goal, logs));
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

/// Once every Ticket has landed: open the Spec PR as a draft, run the Spec
/// review, mark the Spec PR ready and take it to `goal`, as [`run`] does.
fn open_and_review(
    spec: &IssueUrl,
    worktree: Worktree,
    base: &str,
    goal: Goal,
    logs: &Logs,
) -> Result<Reached, FailedRun> {
    let pr_url = open_spec_pr(spec, worktree.branch(), base)?;
    let mut log = logs.path(SPEC_REVIEW);
    match review_and_deliver(spec, &worktree, base, &pr_url, goal, logs, &mut log) {
        Ok(()) => Ok(Reached {
            pr_url,
            goal,
            log: Some(log),
            ticket_lines: Vec::new(),
        }),
        Err(error) => Err(failed_run::fail(spec, worktree, base, &log, error)),
    }
}

/// How a Ticket's Run in this Spec run ended.
enum TicketOutcome {
    /// Its PR, as the Run printed it.
    Landed(Option<String>),
    /// Why, and its session log, as the Run reported them.
    Failed { cause: String, log: Option<String> },
    /// Ended by an interrupt passed on to it.
    Interrupted,
}

/// Push the Spec branch, then keep up to `parallel` Tickets running, each
/// time one ends starting ready ones, lowest number first, until none is
/// ready and none is running. Returns a line on each Ticket that landed or is
/// not done, none if the Spec branch could not be pushed, and whether every
/// Ticket is done: if not, it fails, after putting those lines on stderr
/// unless an interrupt or another error ended it first.
fn land_tickets(
    spec: &IssueUrl,
    mut tickets: Vec<Ticket>,
    worktree: &Worktree,
    parallel: NonZeroUsize,
) -> (Vec<String>, Result<()>) {
    if let Err(error) = worktree.push() {
        return (Vec::new(), Err(error));
    }
    let mut outcomes = BTreeMap::new();
    let landing = run_ready_tickets(
        spec,
        &mut tickets,
        &mut outcomes,
        worktree.branch(),
        parallel,
    );
    let lines = summarize(&tickets, &outcomes);
    let landed = landing.and_then(|()| {
        let not_done: Vec<String> = tickets
            .iter()
            .filter(|ticket| ticket.is_open)
            .map(|ticket| format!("#{}", ticket.number))
            .collect();
        if not_done.is_empty() {
            return Ok(());
        }
        for line in &lines {
            progress::step(line);
        }
        bail!("Tickets not done: {}", not_done.join(", "));
    });
    (lines, landed)
}

/// Keep up to `parallel` ready Tickets running, lowest number first, until
/// none is ready and none is running, keeping `tickets` as last read and each
/// Ticket's outcome in `outcomes`. An interrupt, or an error other than a
/// Ticket failing, stops any more from starting, and fails this once those
/// running have ended.
fn run_ready_tickets(
    spec: &IssueUrl,
    tickets: &mut Vec<Ticket>,
    outcomes: &mut BTreeMap<u64, TicketOutcome>,
    spec_branch: &str,
    parallel: NonZeroUsize,
) -> Result<()> {
    let (ended, endings) = mpsc::channel();
    let mut running = HashSet::new();
    let mut error = None;
    loop {
        while error.is_none() && !interrupt::requested() && running.len() < parallel.get() {
            let Some(ticket) = next_ready(tickets, outcomes, &running) else {
                break;
            };
            match start_ticket(spec, ticket, spec_branch, ended.clone()) {
                Ok(()) => {
                    running.insert(ticket);
                }
                Err(start_error) => error = Some(start_error),
            }
        }
        if running.is_empty() {
            break;
        }
        let (ticket, result) = endings
            .recv()
            .expect("a running Ticket's thread holds a sender");
        running.remove(&ticket);
        // Once there is an error, the rest are only waited for.
        let reread = result.and_then(|outcome| {
            outcomes.insert(ticket, outcome);
            github::tickets(spec)
        });
        match reread {
            Ok(reread) => *tickets = reread,
            Err(reread_error) => {
                error.get_or_insert(reread_error);
            }
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

/// The lowest-numbered Ticket that is open, not an Unready Ticket, has no
/// sub-issues, has every blocker closed and none `running`, isn't `running`
/// and has no outcome in `outcomes` yet. A blocker's Run closes its issue
/// before it ends, so a closed blocker may still be running.
fn next_ready(
    tickets: &[Ticket],
    outcomes: &BTreeMap<u64, TicketOutcome>,
    running: &HashSet<u64>,
) -> Option<u64> {
    tickets
        .iter()
        .filter(|ticket| {
            ticket.is_open
                && !ticket.has_sub_issues
                && ticket.open_blockers.is_empty()
                && !ticket
                    .blockers
                    .iter()
                    .any(|blocker| running.contains(blocker))
                && !outcomes.contains_key(&ticket.number)
                && !running.contains(&ticket.number)
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
fn summarize(tickets: &[Ticket], outcomes: &BTreeMap<u64, TicketOutcome>) -> Vec<String> {
    let mut sorted: Vec<&Ticket> = tickets.iter().collect();
    sorted.sort_by_key(|ticket| ticket.number);
    let mut lines = Vec::new();
    for ticket in sorted {
        let number = ticket.number;
        let landed = match outcomes.get(&number) {
            Some(TicketOutcome::Landed(Some(pr_url))) => Some(format!("landed with {pr_url}")),
            Some(TicketOutcome::Landed(None)) => Some("landed".to_string()),
            _ => None,
        };
        let line = if !ticket.is_open {
            match landed {
                Some(landed) => landed,
                None => continue,
            }
        } else if let Some(TicketOutcome::Failed { cause, log }) = outcomes.get(&number) {
            match log {
                Some(log) => format!("failed: {cause} (session log: {log})"),
                None => format!("failed: {cause}"),
            }
        } else if let Some(TicketOutcome::Interrupted) = outcomes.get(&number) {
            "interrupted".to_string()
        } else if let Some(landed) = landed {
            format!("{landed}, but is still open")
        } else if let Some(label) = unready_label(ticket) {
            format!("unready: labelled {label}")
        } else if ticket.has_sub_issues {
            "unready: has sub-issues".to_string()
        } else if let Some(cycle) = cycle_through(number, tickets) {
            let cycle: Vec<String> = cycle.iter().map(|number| format!("#{number}")).collect();
            format!("in a cycle: {}", cycle.join(" blocked by "))
        } else if ticket.open_blockers.is_empty() {
            // Ready, but the Spec run ended before it could start.
            "not started".to_string()
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
        lines.push(format!("#{number} {line}"));
    }
    lines
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

/// Start Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory, and a thread that sends
/// `number` and how it ended on `ended`.
fn start_ticket(
    spec: &IssueUrl,
    number: u64,
    spec_branch: &str,
    ended: Sender<(u64, Result<TicketOutcome>)>,
) -> Result<()> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
    thread::spawn(move || {
        let _ = ended.send((number, finish_ticket(number, child)));
    });
    Ok(())
}

/// Relay the stderr of Ticket `number`'s Run `child` with a `#<number>: `
/// prefix until it exits. An interrupt is passed on to the child, which is
/// waited for as it goes down its Failed run path, and its outcome is
/// `Interrupted`. Otherwise it landed if it exits 0, and failed otherwise,
/// with the cause and session log it ended on.
fn finish_ticket(number: u64, mut child: Child) -> Result<TicketOutcome> {
    // Relay on its own thread, so this one can watch for an interrupt.
    let stderr = child.stderr.take().context("no stderr from the Run")?;
    // A failed Run ends on its error, then its session log if it has one.
    let relay = thread::spawn(move || -> std::io::Result<[Option<String>; 2]> {
        let mut last_lines: [Option<String>; 2] = [None, None];
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            let line = line
                .strip_prefix("thirdshift: ")
                .unwrap_or(&line)
                .to_string();
            progress::step(format_args!("#{number}: {line}"));
            last_lines = [last_lines[1].take(), Some(line)];
        }
        Ok(last_lines)
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
    let last_lines = relay
        .join()
        .map_err(|_| anyhow!("the relay of the Run for #{number} panicked"))?
        .with_context(|| format!("could not read the Run for #{number}"))?;
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .context("no stdout from the Run")?
        .read_to_string(&mut stdout)
        .with_context(|| format!("could not read the Run for #{number}"))?;
    if status.success() {
        progress::step(format_args!("#{number} landed"));
        return Ok(TicketOutcome::Landed(
            stdout.lines().last().map(String::from),
        ));
    }
    if interrupt::requested() {
        progress::step(format_args!("#{number} interrupted"));
        return Ok(TicketOutcome::Interrupted);
    }
    progress::step(format_args!("#{number} failed"));
    let [before, last] = last_lines;
    let (cause, log) = match last {
        Some(last) => match last.strip_prefix(failed_run::SESSION_LOG) {
            Some(log) => (before, Some(log.to_string())),
            None => (Some(last), None),
        },
        None => (None, None),
    };
    Ok(TicketOutcome::Failed {
        cause: cause.unwrap_or_else(|| format!("the Run {status}")),
        log,
    })
}

/// The Spec PR from `spec_branch` into `base`: the open one if there is one,
/// else a new draft titled from the Spec that closes it.
fn open_spec_pr(spec: &IssueUrl, spec_branch: &str, base: &str) -> Result<String> {
    if interrupt::requested() {
        bail!("interrupted");
    }
    if let Some(pr) = github::pull_request_for(spec, spec_branch)?.filter(|pr| pr.is_open()) {
        return Ok(pr.url);
    }
    progress::step(format_args!("opening the Spec PR into {base} as a draft"));
    let title = github::issue_title(spec)?;
    let body = format!(
        "The work on Spec #{number}, gathered from its Tickets on {spec_branch}.\n\n\
         Closes #{number}\n",
        number = spec.number
    );
    github::create_draft_pr(spec, spec_branch, base, &title, &body)
}

/// Bring the Spec branch in `worktree` up to date with the Tickets landed on
/// origin, and run the Spec review on it. Then push the Spec branch, for any
/// commit the session left unpushed, mark the Spec PR `pr_url` into `base`
/// ready, and [`run::deliver`] it to `goal`, with the Spec as the issue.
/// `log` is left at the most recent session's log.
fn review_and_deliver(
    spec: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    pr_url: &str,
    goal: Goal,
    logs: &Logs,
    log: &mut PathBuf,
) -> Result<()> {
    let spec_branch = worktree.branch();
    worktree.fast_forward_to_origin()?;
    let plugin = Plugin::write()?;
    let sessions = Sessions {
        logs,
        worktree: worktree.path(),
        plugin_dir: plugin.path(),
    };
    let mut run_session = |kind: &str, prompt: &str| sessions.run(kind, prompt, log);
    run_session(
        SPEC_REVIEW,
        &prompt::spec_review(spec, base, spec_branch, pr_url),
    )?;
    worktree.push()?;
    let pr = run::mark_pr_ready(spec, spec_branch, base)?;
    run::deliver(spec, worktree, base, &pr, goal, &mut run_session)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(number: u64, is_open: bool, blockers: &[u64], open_blockers: &[u64]) -> Ticket {
        Ticket {
            number,
            is_open,
            labels: Vec::new(),
            has_sub_issues: false,
            blockers: blockers.to_vec(),
            open_blockers: open_blockers.to_vec(),
        }
    }

    #[test]
    fn a_ticket_waits_for_a_blocker_whose_run_closed_its_issue_but_has_not_ended() {
        // #22's Run closed #22 but is still cleaning up; #21 has landed.
        let tickets = [
            ticket(21, false, &[], &[]),
            ticket(22, false, &[], &[]),
            ticket(23, true, &[21, 22], &[]),
        ];
        let outcomes = BTreeMap::from([(21, TicketOutcome::Landed(None))]);

        assert_eq!(next_ready(&tickets, &outcomes, &HashSet::from([22])), None);
        assert_eq!(next_ready(&tickets, &outcomes, &HashSet::new()), Some(23));
    }
}
