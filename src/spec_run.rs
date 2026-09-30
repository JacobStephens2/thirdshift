//! A Spec run: each of a Spec's Tickets, in dependency order, taken by a
//! Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use crate::args;
use crate::failed_run::{self, FailedRun};
use crate::github::{self, PullRequest, Ticket};
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

/// The markers around the Tickets checklist in the Spec PR's body, so it can
/// be replaced without touching the text around it.
const CHECKLIST_START: &str = "<!-- thirdshift:tickets -->";
const CHECKLIST_END: &str = "<!-- /thirdshift:tickets -->";

/// Take the Spec `spec`, whose Tickets were last read as `tickets`, from
/// its Spec branch, checked out in `worktree`, to a Spec PR into `base`,
/// ready for review. The Spec branch is pushed before any Ticket starts, and
/// the Tickets run one at a time, the graph read again after each. The Spec
/// PR is opened as a draft once the first Ticket lands, or taken as it is if
/// it is already open, and its Tickets checklist rewritten as each Ticket
/// starts and ends. A Ticket that fails stops only the Tickets it blocks;
/// with any Ticket not done, this is a Failed spec run, which leaves the
/// Spec PR, if there is one, a draft. An interrupt ends it too, once the
/// running Ticket's Run has ended, starting nothing more. Once every Ticket
/// has landed, the Spec review, logged in `logs`, reviews the Spec branch,
/// then the Tickets checklist is put back in the Spec PR's body and the Spec
/// PR is marked ready; a Spec review that fails goes through the Failed run
/// path. The worktree is cleaned up when this returns, or kept by the Failed
/// run path if its work did not reach origin.
pub fn run(
    spec: &IssueUrl,
    tickets: Vec<Ticket>,
    worktree: Worktree,
    base: &str,
    logs: &Logs,
) -> Result<Reached, FailedRun> {
    let mut spec_pr = draft_spec_pr(spec, worktree.branch())?;
    let checklist = match land_tickets(spec, tickets, &worktree, base, &mut spec_pr) {
        Ok(checklist) => checklist,
        Err(error) => {
            return Err(FailedRun {
                pr_url: spec_pr.map(|pr| pr.url),
                ..FailedRun::from(error)
            });
        }
    };
    let pr_url = match spec_pr {
        Some(pr) => pr.url,
        None => open_spec_pr(spec, worktree.branch(), base, &checklist)?.url,
    };
    let mut log = logs.path(SPEC_REVIEW);
    if let Err(error) = review(spec, &worktree, base, &pr_url, &checklist, logs, &mut log) {
        return Err(failed_run::fail(spec, worktree, base, &log, error));
    }
    Ok(Reached {
        pr_url,
        goal: Goal::ReadyForReview,
        log: Some(log),
    })
}

/// Where a Ticket's Run in this Spec run stands.
enum TicketOutcome {
    /// It has started and not yet ended.
    Running,
    /// Its PR, as the Run printed it.
    Landed(Option<String>),
    /// Why, and its session log, as the Run reported them.
    Failed { cause: String, log: Option<String> },
    /// It was interrupted, along with this Spec run.
    Interrupted,
}

/// Push the Spec branch, then run each ready Ticket, lowest number first,
/// until none is ready, keeping the Tickets checklist of `spec_pr` up to
/// date, and opening it into `base` once a Ticket lands if there is none.
/// Returns the last Tickets checklist. Fails, after a line on each Ticket
/// that landed or is not done, if any Ticket is not done.
fn land_tickets(
    spec: &IssueUrl,
    mut tickets: Vec<Ticket>,
    worktree: &Worktree,
    base: &str,
    spec_pr: &mut Option<PullRequest>,
) -> Result<String> {
    let spec_branch = worktree.branch();
    worktree.push()?;
    let mut outcomes = BTreeMap::new();
    while let Some(ticket) = next_ready(&tickets, &outcomes) {
        if interrupt::requested() {
            bail!("interrupted");
        }
        outcomes.insert(ticket, TicketOutcome::Running);
        if let Some(pr) = spec_pr {
            write_checklist(spec, pr, &checklist(&tickets, &outcomes))?;
        }
        let outcome = run_ticket(spec, ticket, spec_branch)?;
        let interrupted = matches!(outcome, TicketOutcome::Interrupted);
        let landed = matches!(outcome, TicketOutcome::Landed(_));
        outcomes.insert(ticket, outcome);
        tickets = github::tickets(spec)?;
        let list = checklist(&tickets, &outcomes);
        match spec_pr {
            Some(pr) => write_checklist(spec, pr, &list)?,
            None if landed => *spec_pr = Some(open_spec_pr(spec, spec_branch, base, &list)?),
            None => {}
        }
        if interrupted {
            bail!("interrupted");
        }
    }
    if interrupt::requested() {
        bail!("interrupted");
    }
    let not_done: Vec<String> = tickets
        .iter()
        .filter(|ticket| ticket.is_open)
        .map(|ticket| format!("#{}", ticket.number))
        .collect();
    if not_done.is_empty() {
        return Ok(checklist(&tickets, &outcomes));
    }
    report(&tickets, &outcomes);
    bail!("Tickets not done: {}", not_done.join(", "));
}

/// The lowest-numbered Ticket that is open, not an Unready Ticket, has no
/// sub-issues, has every blocker closed and has no outcome in `outcomes` yet.
fn next_ready(tickets: &[Ticket], outcomes: &BTreeMap<u64, TicketOutcome>) -> Option<u64> {
    tickets
        .iter()
        .filter(|ticket| {
            ticket.is_open
                && !ticket.has_sub_issues
                && ticket.open_blockers.is_empty()
                && !outcomes.contains_key(&ticket.number)
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
fn report(tickets: &[Ticket], outcomes: &BTreeMap<u64, TicketOutcome>) {
    for ticket in by_number(tickets) {
        let number = ticket.number;
        if !ticket.is_open && !outcomes.contains_key(&number) {
            continue;
        }
        let standing = standing(ticket, tickets, outcomes);
        match outcomes.get(&number) {
            Some(TicketOutcome::Failed { log: Some(log), .. }) if ticket.is_open => {
                progress::step(format_args!("#{number} {standing} (session log: {log})"));
            }
            _ => progress::step(format_args!("#{number} {standing}")),
        }
    }
}

/// `tickets`, lowest number first.
fn by_number(tickets: &[Ticket]) -> Vec<&Ticket> {
    let mut sorted: Vec<&Ticket> = tickets.iter().collect();
    sorted.sort_by_key(|ticket| ticket.number);
    sorted
}

/// Where `ticket`, one of `tickets`, stands in this Spec run, as in
/// "#21 <standing>": done, with its PR if it landed in this Spec run, or why
/// it is not done.
fn standing(
    ticket: &Ticket,
    tickets: &[Ticket],
    outcomes: &BTreeMap<u64, TicketOutcome>,
) -> String {
    let number = ticket.number;
    let landed = match outcomes.get(&number) {
        Some(TicketOutcome::Landed(Some(pr_url))) => Some(format!("landed with {pr_url}")),
        Some(TicketOutcome::Landed(None)) => Some("landed".to_string()),
        _ => None,
    };
    if !ticket.is_open {
        return landed.unwrap_or_else(|| "done".to_string());
    }
    match outcomes.get(&number) {
        Some(TicketOutcome::Running) => return "running".to_string(),
        Some(TicketOutcome::Interrupted) => return "interrupted".to_string(),
        Some(TicketOutcome::Failed { cause, .. }) => return format!("failed: {cause}"),
        _ => {}
    }
    if let Some(landed) = landed {
        format!("{landed}, but is still open")
    } else if let Some(label) = unready_label(ticket) {
        format!("unready: labelled {label}")
    } else if ticket.has_sub_issues {
        "unready: has sub-issues".to_string()
    } else if let Some(cycle) = cycle_through(number, tickets) {
        let cycle: Vec<String> = cycle.iter().map(|number| format!("#{number}")).collect();
        format!("in a cycle: {}", cycle.join(" blocked by "))
    } else if ticket.open_blockers.is_empty() {
        "to do".to_string()
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
    }
}

/// The Tickets checklist, between its markers: a line on each of
/// `tickets`, lowest number first, ticked if it is done, with where it
/// stands.
fn checklist(tickets: &[Ticket], outcomes: &BTreeMap<u64, TicketOutcome>) -> String {
    let mut list = format!("{CHECKLIST_START}\n## Tickets\n\n");
    for ticket in by_number(tickets) {
        let tick = if ticket.is_open { ' ' } else { 'x' };
        list += &format!(
            "- [{tick}] #{} {}\n",
            ticket.number,
            standing(ticket, tickets, outcomes)
        );
    }
    list + CHECKLIST_END + "\n"
}

/// `body` with its Tickets checklist, the text from its first start marker
/// to the next end marker, replaced by `checklist`, or with `checklist`
/// appended if it has no such pair of markers.
fn with_checklist(body: &str, checklist: &str) -> String {
    if let Some(start) = body.find(CHECKLIST_START)
        && let Some(end) = body[start..].find(CHECKLIST_END)
    {
        let end = start + end + CHECKLIST_END.len();
        let end = if body[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        return format!("{}{checklist}{}", &body[..start], &body[end..]);
    }
    if body.is_empty() {
        return checklist.to_string();
    }
    format!("{}\n\n{checklist}", body.trim_end())
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
/// `#<number>: ` prefix. An interrupt is passed on to the child, which is
/// waited for as it goes down its Failed run path, and then it was
/// interrupted. Otherwise it landed if it exits 0, and failed otherwise, with
/// the cause and session log it ended on.
fn run_ticket(spec: &IssueUrl, number: u64, spec_branch: &str) -> Result<TicketOutcome> {
    progress::step(format_args!("starting #{number}"));
    let ticket = spec.sibling(number);
    let mut child = Command::new(std::env::current_exe().context("no thirdshift executable")?)
        .args([args::SPEC_BRANCH, spec_branch, &ticket.url])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("could not start the Run for #{number}"))?;
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

/// The open Spec PR from `spec_branch`, if there is one, converted back to a
/// draft while the Tickets run.
fn draft_spec_pr(spec: &IssueUrl, spec_branch: &str) -> Result<Option<PullRequest>> {
    let Some(pr) = github::pull_request_for(spec, spec_branch)?.filter(|pr| pr.is_open()) else {
        return Ok(None);
    };
    if !pr.is_draft {
        github::convert_to_draft(spec, spec_branch)?;
    }
    Ok(Some(pr))
}

/// Open the Spec PR from `spec_branch` into `base` as a draft, titled from
/// the Spec, closing it, with `checklist` as its Tickets checklist.
fn open_spec_pr(
    spec: &IssueUrl,
    spec_branch: &str,
    base: &str,
    checklist: &str,
) -> Result<PullRequest> {
    progress::step(format_args!("opening the Spec PR into {base} as a draft"));
    let title = github::issue_title(spec)?;
    let body = format!(
        "The work on Spec #{number}, gathered from its Tickets on {spec_branch}.\n\n\
         Closes #{number}\n\n\
         {checklist}",
        number = spec.number
    );
    github::create_draft_pr(spec, spec_branch, base, &title, &body)?;
    github::pull_request_for(spec, spec_branch)?.context("the Spec PR just opened is not found")
}

/// Put `checklist` in the body of the Spec PR `pr`, in place of its Tickets
/// checklist, leaving the rest of the body as it is.
fn write_checklist(spec: &IssueUrl, pr: &PullRequest, checklist: &str) -> Result<()> {
    let body = github::pr_body(spec, pr.number)?;
    let updated = with_checklist(&body, checklist);
    if updated != body {
        progress::step("updating the Spec PR's Tickets checklist");
        github::set_pr_body(spec, pr.number, &updated)?;
    }
    Ok(())
}

/// Bring the Spec branch in `worktree` up to date with the Tickets landed on
/// origin, and run the Spec review on it, pointing `log` at its session's
/// log. Then push the Spec branch, for any commit the session left unpushed,
/// put `checklist` back in the body the session wrote for the Spec PR
/// `pr_url` into `base`, and mark it ready.
fn review(
    spec: &IssueUrl,
    worktree: &Worktree,
    base: &str,
    pr_url: &str,
    checklist: &str,
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
    let prompt = prompt::spec_review(spec, base, spec_branch, pr_url);
    sessions.run(SPEC_REVIEW, &prompt, log)?;
    worktree.push()?;
    let pr = github::pull_request_for(spec, spec_branch)?.context("no PR found")?;
    write_checklist(spec, &pr, checklist)?;
    run::mark_pr_ready(spec, spec_branch, base)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(number: u64, is_open: bool, open_blockers: &[u64]) -> Ticket {
        Ticket {
            number,
            is_open,
            labels: Vec::new(),
            has_sub_issues: false,
            open_blockers: open_blockers.to_vec(),
        }
    }

    #[test]
    fn the_checklist_ticks_done_tickets_and_says_where_each_other_stands() {
        let mut unready = ticket(26, true, &[]);
        unready.labels = vec!["needs-info".to_string()];
        let tickets = [
            ticket(23, true, &[]),
            ticket(21, false, &[]),
            ticket(22, false, &[]),
            ticket(24, true, &[23]),
            ticket(25, true, &[]),
            unready,
            ticket(27, true, &[]),
        ];
        let outcomes = BTreeMap::from([
            (
                21,
                TicketOutcome::Landed(Some("https://x/pull/1".to_string())),
            ),
            (
                23,
                TicketOutcome::Failed {
                    cause: "claude exited 1".to_string(),
                    log: Some("/logs/23.jsonl".to_string()),
                },
            ),
            (25, TicketOutcome::Running),
        ]);

        assert_eq!(
            checklist(&tickets, &outcomes),
            "<!-- thirdshift:tickets -->\n\
             ## Tickets\n\
             \n\
             - [x] #21 landed with https://x/pull/1\n\
             - [x] #22 done\n\
             - [ ] #23 failed: claude exited 1\n\
             - [ ] #24 blocked by #23\n\
             - [ ] #25 running\n\
             - [ ] #26 unready: labelled needs-info\n\
             - [ ] #27 to do\n\
             <!-- /thirdshift:tickets -->\n"
        );
    }

    const LIST: &str = "<!-- thirdshift:tickets -->\nnew\n<!-- /thirdshift:tickets -->\n";

    #[test]
    fn the_checklist_replaces_the_one_between_the_markers_and_leaves_the_rest() {
        let body = "Intro.\n\n<!-- thirdshift:tickets -->\nold\n<!-- /thirdshift:tickets -->\n\nCloses #20\n";

        assert_eq!(
            with_checklist(body, LIST),
            format!("Intro.\n\n{LIST}\nCloses #20\n")
        );
    }

    #[test]
    fn the_checklist_is_appended_when_the_markers_are_gone() {
        assert_eq!(
            with_checklist("A rewritten body.\n\nCloses #20\n", LIST),
            format!("A rewritten body.\n\nCloses #20\n\n{LIST}")
        );
        assert_eq!(with_checklist("", LIST), LIST);
    }

    #[test]
    fn the_checklist_is_appended_when_only_one_marker_is_left() {
        let body = "Text\n<!-- /thirdshift:tickets -->\n<!-- thirdshift:tickets -->\nmore";

        assert_eq!(with_checklist(body, LIST), format!("{body}\n\n{LIST}"));
    }
}
