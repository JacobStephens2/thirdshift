//! A Spec run: each of a Spec's Tickets, in dependency order and several at
//! once, taken by a Ticket's Run, a Merge run into the Spec branch in a child `thirdshift`
//! (ADR-0006), then the Spec review and the Spec PR from the Spec branch into
//! the Base branch, kept mergeable and green like a Run's PR, and Self-merged
//! when the Spec run was asked to merge.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::mpsc::{self, Sender};
use std::thread;

use anyhow::{Context, Result, bail};

use crate::base_fix::BaseFixAsk;
use crate::child_run::{self, Ended, Handle, Kind};
use crate::delivery::{Delivery, Opening};
use crate::failed_run::FailedRun;
use crate::github::{self, PullRequest, Ticket};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;
use crate::prompt;
use crate::run::Reached;
use crate::worktree::Worktree;

/// The Spec review session's kind, in its progress lines and log name.
const SPEC_REVIEW: &str = "spec-review";

/// The markers around the Tickets checklist in the Spec PR's body, so it can
/// be replaced without touching the text around it.
const CHECKLIST_START: &str = "<!-- thirdshift:tickets -->";
const CHECKLIST_END: &str = "<!-- /thirdshift:tickets -->";

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
    let mut spec_pr = draft_spec_pr(spec, worktree.branch())?;
    let (ticket_lines, landed) = land_tickets(
        spec,
        tickets,
        &worktree,
        base,
        parallel,
        &delivery.base_fix.ask_of_tickets(),
        &mut spec_pr,
    );
    let ended = match landed {
        Ok(checklist) => review_and_deliver(worktree, spec_pr, &checklist, delivery),
        Err(error) => Err(FailedRun {
            pr_url: spec_pr.map(|pr| pr.url),
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

/// Once every Ticket has landed: open the Spec PR as a draft with
/// `checklist` if `spec_pr` is none, then take it to its goal by `delivery`,
/// opening with the Spec review, as [`run`] does.
fn review_and_deliver(
    worktree: Worktree,
    spec_pr: Option<PullRequest>,
    checklist: &str,
    delivery: Delivery,
) -> Result<Reached, FailedRun> {
    let (spec, base) = (delivery.issue, delivery.base);
    let spec_pr = match spec_pr {
        Some(pr) => pr,
        None => open_spec_pr(spec, worktree.branch(), base, checklist)?,
    };
    let opening = Opening {
        kind: SPEC_REVIEW,
        prompt: prompt::spec_review(spec, base, worktree.branch(), &spec_pr.url),
        catch_up_from_origin: true,
    };
    // The Spec review may have rewritten the body without the checklist.
    let delivered = delivery.deliver(worktree, opening, || {
        write_checklist(spec, &spec_pr, checklist)
    });
    if delivered.is_err() {
        write_checklist_or_warn(spec, &spec_pr, checklist);
    }
    delivered
}

/// Where a Ticket's Run in this Spec run stands.
enum TicketOutcome {
    /// It has started and not yet ended.
    Running,
    /// It has ended, as its Run told: it landed if it reached its goal.
    Ended(Ended),
}

/// Push the Spec branch, then keep up to `parallel` Tickets running, each
/// time one ends starting ready ones, lowest number first, until none is
/// ready and none is running, keeping the Tickets checklist of `spec_pr` up
/// to date as far as GitHub lets it, and opening it into `base` once a Ticket
/// lands if there is none. Returns a line on each Ticket that landed or is
/// not done, none if the Spec branch could not be pushed, and the last
/// Tickets checklist if every Ticket is done: if not, it fails, after putting
/// those lines on stderr unless an interrupt or another error ended it first.
/// Each Ticket's Run may start a Base fix if `base_fix` allows one.
fn land_tickets(
    spec: &IssueUrl,
    mut tickets: Vec<Ticket>,
    worktree: &Worktree,
    base: &str,
    parallel: NonZeroUsize,
    base_fix: &BaseFixAsk,
    spec_pr: &mut Option<PullRequest>,
) -> (Vec<String>, Result<String>) {
    if let Err(error) = worktree.push() {
        return (Vec::new(), Err(error));
    }
    let mut outcomes = BTreeMap::new();
    let landing = run_ready_tickets(
        spec,
        &mut tickets,
        &mut outcomes,
        worktree.branch(),
        base,
        parallel,
        base_fix,
        spec_pr,
    );
    let lines = summarize(&tickets, &outcomes);
    let landed = landing.and_then(|()| {
        let not_done: Vec<String> = tickets
            .iter()
            .filter(|ticket| ticket.is_open)
            .map(|ticket| format!("#{}", ticket.number))
            .collect();
        if not_done.is_empty() {
            return Ok(checklist(&tickets, &outcomes));
        }
        for line in &lines {
            progress::step(line);
        }
        bail!("Tickets not done: {}", not_done.join(", "));
    });
    (lines, landed)
}

/// Keep up to `parallel` ready Tickets running, lowest number first, until
/// none is ready and none is running, keeping `tickets` as last read, each
/// Ticket's outcome in `outcomes`, and the Tickets checklist of `spec_pr`,
/// opened into `base` once a Ticket lands, up to date. An interrupt, or an
/// error other than a Ticket failing, stops any more from starting, and
/// fails this once those running have ended. Each Ticket's Run may start a
/// Base fix if `base_fix` allows one.
#[allow(clippy::too_many_arguments)]
fn run_ready_tickets(
    spec: &IssueUrl,
    tickets: &mut Vec<Ticket>,
    outcomes: &mut BTreeMap<u64, TicketOutcome>,
    spec_branch: &str,
    base: &str,
    parallel: NonZeroUsize,
    base_fix: &BaseFixAsk,
    spec_pr: &mut Option<PullRequest>,
) -> Result<()> {
    let (ended, endings) = mpsc::channel();
    let mut running = HashSet::new();
    let mut error = None;
    loop {
        let mut started = false;
        while error.is_none() && !interrupt::requested() && running.len() < parallel.get() {
            let Some(ticket) = next_ready(tickets, outcomes, &running) else {
                break;
            };
            match start_ticket(spec, ticket, spec_branch, base_fix, ended.clone()) {
                Ok(()) => {
                    running.insert(ticket);
                    outcomes.insert(ticket, TicketOutcome::Running);
                    started = true;
                }
                Err(start_error) => error = Some(start_error),
            }
        }
        if started && let Some(pr) = spec_pr {
            write_checklist_or_warn(spec, pr, &checklist(tickets, outcomes));
        }
        if running.is_empty() {
            break;
        }
        let (ticket, result) = endings
            .recv()
            .expect("a running Ticket's thread holds a sender");
        running.remove(&ticket);
        // Once there is an error, the rest are only waited for, but each
        // still has its line in the checklist, from the last graph read.
        let outcome = result.unwrap_or_else(|finish_error| {
            let cause = format!("{finish_error:#}");
            error.get_or_insert(finish_error);
            TicketOutcome::Ended(Ended::Failed { cause, log: None })
        });
        let landed = matches!(outcome, TicketOutcome::Ended(Ended::Reached { .. }));
        outcomes.insert(ticket, outcome);
        match github::tickets(spec) {
            Ok(reread) => *tickets = reread,
            Err(reread_error) => {
                error.get_or_insert(reread_error);
            }
        }
        let list = checklist(tickets, outcomes);
        match spec_pr {
            Some(pr) => write_checklist_or_warn(spec, pr, &list),
            None if landed => match open_spec_pr(spec, spec_branch, base, &list) {
                Ok(pr) => *spec_pr = Some(pr),
                Err(open_error) => {
                    error.get_or_insert(open_error);
                }
            },
            None => {}
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
                && ticket.labels.unready().is_none()
        })
        .map(|ticket| ticket.number)
        .min()
}

/// A line on each Ticket that landed in this Spec run, with its PR, and on
/// each Ticket not done, with why, lowest number first.
fn summarize(tickets: &[Ticket], outcomes: &BTreeMap<u64, TicketOutcome>) -> Vec<String> {
    let mut lines = Vec::new();
    for ticket in by_number(tickets) {
        let number = ticket.number;
        if !ticket.is_open && !outcomes.contains_key(&number) {
            continue;
        }
        let standing = standing(ticket, tickets, outcomes);
        lines.push(match outcomes.get(&number) {
            Some(TicketOutcome::Ended(Ended::Failed { log: Some(log), .. })) if ticket.is_open => {
                format!("#{number} {standing} (session log: {log})")
            }
            _ => format!("#{number} {standing}"),
        });
    }
    lines
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
        Some(TicketOutcome::Ended(Ended::Reached { pr_url, base_fix })) => {
            let mut landed = "landed".to_string();
            if let Some(pr_url) = pr_url {
                landed += &format!(" with {pr_url}");
            }
            if let Some(base_fix) = base_fix {
                landed += &format!(", after Base fix {base_fix}");
            }
            Some(landed)
        }
        _ => None,
    };
    if !ticket.is_open {
        return landed.unwrap_or_else(|| "done".to_string());
    }
    match outcomes.get(&number) {
        Some(TicketOutcome::Running) => return "running".to_string(),
        Some(TicketOutcome::Ended(Ended::Interrupted)) => return "interrupted".to_string(),
        Some(TicketOutcome::Ended(Ended::Failed { cause, .. })) => {
            return format!("failed: {cause}");
        }
        _ => {}
    }
    if let Some(landed) = landed {
        format!("{landed}, but is still open")
    } else if let Some(label) = ticket.labels.unready() {
        format!("unready: labelled {label}")
    } else if ticket.has_sub_issues {
        "unready: has sub-issues".to_string()
    } else if let Some(cycle) = cycle_through(number, tickets) {
        let cycle: Vec<String> = cycle.iter().map(|number| format!("#{number}")).collect();
        format!("in a cycle: {}", cycle.join(" blocked by "))
    } else if ticket.open_blockers.is_empty() {
        // Ready, but not started yet, or the Spec run ended before it could.
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

/// Start Ticket `number` of `spec` as a child `thirdshift`, a Merge run into
/// `spec_branch` from the same Launch directory that may start a Base fix if
/// `base_fix` allows one, and a thread that sends `number` and how it ended
/// on `ended`.
fn start_ticket(
    spec: &IssueUrl,
    number: u64,
    spec_branch: &str,
    base_fix: &BaseFixAsk,
    ended: Sender<(u64, Result<TicketOutcome>)>,
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
fn finish_ticket(number: u64, child: Handle) -> Result<TicketOutcome> {
    let ended = child.wait()?;
    progress::step(match ended {
        Ended::Reached { .. } => format!("#{number} landed"),
        Ended::Interrupted => format!("#{number} interrupted"),
        Ended::Failed { .. } => format!("#{number} failed"),
    });
    Ok(TicketOutcome::Ended(ended))
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

/// [`write_checklist`], only warning if it fails: the Spec run goes on
/// without it.
fn write_checklist_or_warn(spec: &IssueUrl, pr: &PullRequest, checklist: &str) {
    if let Err(error) = write_checklist(spec, pr, checklist) {
        progress::step(format_args!(
            "could not update the Spec PR's Tickets checklist: {error:#}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labels::{Label, Labels, READY_FOR_AGENT, UNREADY};

    fn ticket(number: u64, is_open: bool, blockers: &[u64], open_blockers: &[u64]) -> Ticket {
        Ticket {
            number,
            is_open,
            labels: Labels::default(),
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
        let outcomes = BTreeMap::from([(
            21,
            TicketOutcome::Ended(Ended::Reached {
                pr_url: None,
                base_fix: None,
            }),
        )]);

        assert_eq!(next_ready(&tickets, &outcomes, &HashSet::from([22])), None);
        assert_eq!(next_ready(&tickets, &outcomes, &HashSet::new()), Some(23));
    }

    #[test]
    fn each_unready_label_keeps_a_ticket_from_running_and_is_named_in_its_line() {
        for label in UNREADY {
            let mut unready = ticket(21, true, &[], &[]);
            unready.labels = [READY_FOR_AGENT, label]
                .map(Label::name)
                .into_iter()
                .collect();
            let tickets = [unready, ticket(22, true, &[], &[])];
            let outcomes = BTreeMap::new();

            assert_eq!(
                next_ready(&tickets, &outcomes, &HashSet::new()),
                Some(22),
                "{label}"
            );
            assert_eq!(
                summarize(&tickets, &outcomes)[0],
                format!("#21 unready: labelled {label}")
            );
        }
    }

    #[test]
    fn a_failed_tickets_line_says_its_cause() {
        let tickets = [ticket(23, true, &[], &[])];
        let cause = "git fetch origin issue-21 failed: \
                     error: fetching ref refs/remotes/origin/issue-21 failed";
        let outcomes = BTreeMap::from([(
            23,
            TicketOutcome::Ended(Ended::Failed {
                cause: cause.to_string(),
                log: None,
            }),
        )]);

        assert_eq!(
            summarize(&tickets, &outcomes),
            ["#23 failed: git fetch origin issue-21 failed: \
              error: fetching ref refs/remotes/origin/issue-21 failed"]
        );
    }

    #[test]
    fn the_checklist_ticks_done_tickets_and_says_where_each_other_stands() {
        let mut unready = ticket(26, true, &[], &[]);
        unready.labels = ["needs-info"].into_iter().collect();
        let tickets = [
            ticket(23, true, &[], &[]),
            ticket(21, false, &[], &[]),
            ticket(22, false, &[], &[]),
            ticket(24, true, &[23], &[23]),
            ticket(25, true, &[], &[]),
            unready,
            ticket(27, true, &[], &[]),
        ];
        let outcomes = BTreeMap::from([
            (
                21,
                TicketOutcome::Ended(Ended::Reached {
                    pr_url: Some("https://x/pull/1".to_string()),
                    base_fix: None,
                }),
            ),
            (
                23,
                TicketOutcome::Ended(Ended::Failed {
                    cause: "claude exited 1".to_string(),
                    log: Some("/logs/23.jsonl".to_string()),
                }),
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
             - [ ] #27 not started\n\
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
