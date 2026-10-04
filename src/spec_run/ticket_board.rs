//! The Ticket board: a Spec run's Tickets as last read, where each one it
//! started stands, how many may run at once and whether starting has
//! stopped. It says which Ticket to start next, and each Ticket's line and
//! the Tickets checklist; the Spec run's loop starts and waits for the
//! Tickets' Runs, reads the graph, and tells it what happened.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::num::NonZeroUsize;

use crate::child_run::Ended;
use crate::github::Ticket;

/// Where a Ticket this Spec run started stands.
enum Standing {
    /// Its Run has started and not yet ended.
    Running,
    /// Its Run has ended, as it told: it landed if it reached its goal.
    Ended(Ended),
}

/// A Spec run's Tickets, as last read, and where each one it started stands.
pub(super) struct TicketBoard {
    tickets: Vec<Ticket>,
    standings: BTreeMap<u64, Standing>,
    parallel: NonZeroUsize,
    stopped: bool,
}

impl TicketBoard {
    /// A board of `tickets`, none started, running up to `parallel` at once.
    pub(super) fn new(tickets: Vec<Ticket>, parallel: NonZeroUsize) -> Self {
        Self {
            tickets,
            standings: BTreeMap::new(),
            parallel,
            stopped: false,
        }
    }

    /// The Ticket to start next, if any: none once starting has stopped or
    /// `parallel` are running, else the lowest-numbered Ticket that is open,
    /// not an Unready Ticket, has no sub-issues, has every blocker closed and
    /// none running, and was not started in this Spec run. A blocker's Run
    /// closes its issue before it ends, so a closed blocker may still be
    /// running.
    pub(super) fn next_to_start(&self) -> Option<u64> {
        if self.stopped || self.running() >= self.parallel.get() {
            return None;
        }
        self.tickets
            .iter()
            .filter(|ticket| {
                ticket.is_open
                    && !ticket.has_sub_issues
                    && ticket.open_blockers.is_empty()
                    && !ticket
                        .blockers
                        .iter()
                        .any(|blocker| self.is_running(*blocker))
                    && !self.standings.contains_key(&ticket.number)
                    && ticket.labels.unready().is_none()
            })
            .map(|ticket| ticket.number)
            .min()
    }

    /// Ticket `number`'s Run has started.
    pub(super) fn started(&mut self, number: u64) {
        self.standings.insert(number, Standing::Running);
    }

    /// Ticket `number`'s Run has ended, as `ended` tells.
    pub(super) fn ended(&mut self, number: u64, ended: Ended) {
        self.standings.insert(number, Standing::Ended(ended));
    }

    /// Start no more Tickets: those running are still waited for.
    pub(super) fn stop(&mut self) {
        self.stopped = true;
    }

    /// Replace the Tickets with `tickets`, a fresh read of the graph, keeping
    /// where each started one stands.
    pub(super) fn reread(&mut self, tickets: Vec<Ticket>) {
        self.tickets = tickets;
    }

    /// Whether any Ticket's Run is running.
    pub(super) fn any_running(&self) -> bool {
        self.running() > 0
    }

    /// The Tickets not done, the open ones, in the order they were read.
    pub(super) fn not_done(&self) -> Vec<u64> {
        self.tickets
            .iter()
            .filter(|ticket| ticket.is_open)
            .map(|ticket| ticket.number)
            .collect()
    }

    /// A line on each Ticket that landed in this Spec run, with its PR, and
    /// on each Ticket not done, with why, lowest number first.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for ticket in self.by_number() {
            let number = ticket.number;
            if !ticket.is_open && !self.standings.contains_key(&number) {
                continue;
            }
            let standing = self.standing(ticket);
            lines.push(match self.standings.get(&number) {
                Some(Standing::Ended(Ended::Failed { log: Some(log), .. })) if ticket.is_open => {
                    format!("#{number} {standing} (session log: {log})")
                }
                _ => format!("#{number} {standing}"),
            });
        }
        lines
    }

    /// The Tickets checklist: a line on each Ticket, lowest number first,
    /// ticked if it is done, with where it stands.
    pub(super) fn checklist(&self) -> String {
        let mut list = "## Tickets\n\n".to_string();
        for ticket in self.by_number() {
            let tick = if ticket.is_open { ' ' } else { 'x' };
            list += &format!("- [{tick}] #{} {}\n", ticket.number, self.standing(ticket));
        }
        list
    }

    /// How many Tickets' Runs are running.
    fn running(&self) -> usize {
        self.standings
            .values()
            .filter(|standing| matches!(standing, Standing::Running))
            .count()
    }

    /// Whether Ticket `number`'s Run is running.
    fn is_running(&self, number: u64) -> bool {
        matches!(self.standings.get(&number), Some(Standing::Running))
    }

    /// The Tickets, lowest number first.
    fn by_number(&self) -> Vec<&Ticket> {
        let mut sorted: Vec<&Ticket> = self.tickets.iter().collect();
        sorted.sort_by_key(|ticket| ticket.number);
        sorted
    }

    /// Where `ticket` stands in this Spec run, as in "#21 <standing>": done,
    /// with its PR if it landed in this Spec run, or why it is not done.
    fn standing(&self, ticket: &Ticket) -> String {
        let number = ticket.number;
        let landed = match self.standings.get(&number) {
            Some(Standing::Ended(Ended::Reached { pr_url, base_fix })) => {
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
        match self.standings.get(&number) {
            Some(Standing::Running) => return "running".to_string(),
            Some(Standing::Ended(Ended::Interrupted)) => return "interrupted".to_string(),
            Some(Standing::Ended(Ended::Failed { cause, .. })) => {
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
        } else if let Some(cycle) = cycle_through(number, &self.tickets) {
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
                    if self.tickets.iter().any(|ticket| ticket.number == *blocker) {
                        format!("#{blocker}")
                    } else {
                        format!("#{blocker} (outside the Spec)")
                    }
                })
                .collect();
            format!("blocked by {}", blockers.join(", "))
        }
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

    fn board(tickets: Vec<Ticket>, parallel: usize) -> TicketBoard {
        TicketBoard::new(tickets, NonZeroUsize::new(parallel).unwrap())
    }

    fn reached(pr_url: Option<&str>) -> Ended {
        Ended::Reached {
            pr_url: pr_url.map(str::to_string),
            base_fix: None,
        }
    }

    fn failed(cause: &str, log: Option<&str>) -> Ended {
        Ended::Failed {
            cause: cause.to_string(),
            log: log.map(str::to_string),
        }
    }

    #[test]
    fn it_never_offers_more_than_parallel_tickets_at_once() {
        let mut board = board(
            vec![
                ticket(21, true, &[], &[]),
                ticket(22, true, &[], &[]),
                ticket(23, true, &[], &[]),
            ],
            2,
        );

        assert_eq!(board.next_to_start(), Some(21));
        board.started(21);
        assert_eq!(board.next_to_start(), Some(22));
        board.started(22);
        assert_eq!(board.next_to_start(), None);

        board.ended(21, failed("claude exited 1", None));
        assert_eq!(board.next_to_start(), Some(23));
    }

    #[test]
    fn once_stopped_it_offers_none_yet_reports_the_running_ones_until_they_end() {
        let mut board = board(
            vec![ticket(21, true, &[], &[]), ticket(22, true, &[], &[])],
            2,
        );
        board.started(21);

        board.stop();

        assert_eq!(board.next_to_start(), None);
        assert!(board.any_running());
        board.ended(21, Ended::Interrupted);
        assert!(!board.any_running());
        assert_eq!(board.next_to_start(), None);
    }

    #[test]
    fn a_ticket_that_ended_is_not_offered_again_while_it_is_still_open() {
        let mut board = board(
            vec![ticket(21, true, &[], &[]), ticket(22, true, &[], &[])],
            2,
        );
        board.started(21);
        board.started(22);
        board.ended(21, failed("claude exited 1", None));
        board.ended(22, reached(None));

        assert_eq!(board.next_to_start(), None);
    }

    #[test]
    fn a_ticket_waits_for_a_blocker_whose_run_closed_its_issue_but_has_not_ended() {
        // #22's Run closed #22 but is still cleaning up; #21 has landed.
        let mut board = board(
            vec![
                ticket(21, false, &[], &[]),
                ticket(22, false, &[], &[]),
                ticket(23, true, &[21, 22], &[]),
            ],
            2,
        );
        board.started(21);
        board.ended(21, reached(None));
        board.started(22);

        assert_eq!(board.next_to_start(), None);
        board.ended(22, reached(None));
        assert_eq!(board.next_to_start(), Some(23));
    }

    #[test]
    fn a_fresh_read_replaces_the_tickets_and_keeps_where_each_stands() {
        let mut board = board(
            vec![ticket(21, true, &[], &[]), ticket(22, true, &[21], &[21])],
            1,
        );
        board.started(21);
        board.ended(21, reached(Some("https://x/pull/1")));

        board.reread(vec![
            ticket(21, false, &[], &[]),
            ticket(22, true, &[21], &[]),
        ]);

        assert_eq!(
            board.lines(),
            ["#21 landed with https://x/pull/1", "#22 not started"]
        );
        assert_eq!(board.next_to_start(), Some(22));
    }

    #[test]
    fn the_tickets_not_done_are_the_open_ones() {
        let board = board(
            vec![
                ticket(23, true, &[], &[]),
                ticket(21, false, &[], &[]),
                ticket(22, true, &[], &[]),
            ],
            1,
        );

        assert_eq!(board.not_done(), [23, 22]);
    }

    #[test]
    fn each_unready_label_keeps_a_ticket_from_running_and_is_named_in_its_line() {
        for label in UNREADY {
            let mut unready = ticket(21, true, &[], &[]);
            unready.labels = [READY_FOR_AGENT, label]
                .map(Label::name)
                .into_iter()
                .collect();
            let board = board(vec![unready, ticket(22, true, &[], &[])], 1);

            assert_eq!(board.next_to_start(), Some(22), "{label}");
            assert_eq!(board.lines()[0], format!("#21 unready: labelled {label}"));
        }
    }

    #[test]
    fn a_failed_tickets_line_says_its_cause() {
        let mut board = board(vec![ticket(23, true, &[], &[])], 1);
        board.started(23);
        board.ended(
            23,
            failed(
                "git fetch origin issue-21 failed: \
                 error: fetching ref refs/remotes/origin/issue-21 failed",
                None,
            ),
        );

        assert_eq!(
            board.lines(),
            ["#23 failed: git fetch origin issue-21 failed: \
              error: fetching ref refs/remotes/origin/issue-21 failed"]
        );
    }

    #[test]
    fn the_checklist_ticks_done_tickets_and_says_where_each_other_stands() {
        let mut unready = ticket(26, true, &[], &[]);
        unready.labels = ["needs-info"].into_iter().collect();
        let mut board = board(
            vec![
                ticket(23, true, &[], &[]),
                ticket(21, false, &[], &[]),
                ticket(22, false, &[], &[]),
                ticket(24, true, &[23], &[23]),
                ticket(25, true, &[], &[]),
                unready,
                ticket(27, true, &[], &[]),
            ],
            3,
        );
        board.started(21);
        board.ended(21, reached(Some("https://x/pull/1")));
        board.started(23);
        board.ended(23, failed("claude exited 1", Some("/logs/23.jsonl")));
        board.started(25);

        assert_eq!(
            board.checklist(),
            "## Tickets\n\
             \n\
             - [x] #21 landed with https://x/pull/1\n\
             - [x] #22 done\n\
             - [ ] #23 failed: claude exited 1\n\
             - [ ] #24 blocked by #23\n\
             - [ ] #25 running\n\
             - [ ] #26 unready: labelled needs-info\n\
             - [ ] #27 not started\n"
        );
    }
}
