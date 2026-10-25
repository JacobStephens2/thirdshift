//! The search for the Ready issue at the configured end of the issue
//! numbers in a repository, saying why it passed over each issue labelled
//! `ready-for-agent` before it. A Pickup run takes the issue it finds; any
//! run can ask whether a repository has one, against the very same
//! definition. An Architect run also waits for an issue that meets every
//! Ready issue condition except settling.

use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};

use crate::base_fix;
use crate::branch::{self, Started};
use crate::claim;
use crate::git::Git;
use crate::github::{Candidate, GitHub, ListedIssue, Shaping, Ticket};
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Label, READY_FOR_AGENT};
use crate::progress;
use crate::spec_run::{self, Unstartable};
use crate::trust::{self, UntrustedIssue};

/// Which end of the issue numbers a Pickup run takes its Ready issue from.
/// Two hosts working through the same `ready-for-agent` queue from opposite
/// ends collide less often; the reservation still protects either choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickupOrder {
    /// The lowest-numbered Ready issue, the default.
    #[default]
    LowestFirst,
    /// The highest-numbered Ready issue.
    HighestFirst,
}

impl PickupOrder {
    /// The `pickup.order` value and the Pickup flag that ask for `self`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::LowestFirst => "lowest-first",
            Self::HighestFirst => "highest-first",
        }
    }

    /// The order `name`, a `pickup.order` value or Pickup flag word without
    /// its dashes, asks for, if it names one.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "lowest-first" => Some(Self::LowestFirst),
            "highest-first" => Some(Self::HighestFirst),
            _ => None,
        }
    }
}

impl std::fmt::Display for PickupOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The Ready issue a search found, at `order`'s end of the issue numbers.
pub struct ReadyIssue {
    /// The issue, as the search listed it.
    pub listed: ListedIssue,
    /// Whether it is a Spec, an issue with sub-issues.
    pub is_spec: bool,
    /// The Claim generation seen by this search, before reserving the issue.
    pub claim_generation: usize,
}

/// An issue that goes before an Architect run, lowest number first.
pub enum PriorityIssue {
    Ready(ReadyIssue),
    /// Every other Ready issue condition holds, including a Spec having a
    /// Startable Ticket. Pickup still cannot take it.
    Settling(ListedIssue),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Settling {
    Wait,
    Include,
}

/// The Ready issue at `order`'s end of the issue numbers in `repo`, the
/// Launch directory `launch`'s repository, if there is one, with a line on
/// stderr for each issue labelled `ready-for-agent` passed over before it,
/// with why. Nothing is changed, on GitHub or in `launch`.
/// `wait` is the User config's settling window since the latest shaping
/// event.
pub fn first(
    launch: &Git,
    repo: &Repo,
    wait: TimeDelta,
    order: PickupOrder,
) -> Result<Option<ReadyIssue>> {
    Ok(match search(launch, repo, wait, Settling::Wait, order)? {
        Some(PriorityIssue::Ready(ready)) => Some(ready),
        _ => None,
    })
}

/// The lowest-numbered issue that is Ready or would be Ready once settled.
/// Only Architect uses this search; Pickup and Security use `first`.
pub fn before_architect(
    launch: &Git,
    repo: &Repo,
    wait: TimeDelta,
) -> Result<Option<PriorityIssue>> {
    search(
        launch,
        repo,
        wait,
        Settling::Include,
        PickupOrder::LowestFirst,
    )
}

fn search(
    launch: &Git,
    repo: &Repo,
    wait: TimeDelta,
    settling: Settling,
    order: PickupOrder,
) -> Result<Option<PriorityIssue>> {
    let reads = GitHubAndOrigin {
        launch,
        github: GitHub::new(),
    };
    let candidates = reads
        .github
        .open_issues_labelled(&repo.slug(), READY_FOR_AGENT)?;
    Search::new(&reads, candidates, Utc::now(), wait, order, progress::step)
        .first_priority(settling)
}

/// Recheck only the selected issue after Pickup reserves it on origin. A
/// stale selection must not start a Continuation after the winner finished.
/// Refresh Spec status as well: shaping may have changed since the listing.
/// An intervening Claim invalidates selection even if failure restored Ready;
/// the complete timeline's generation works with settling disabled and without
/// relying on clocks or timestamp precision.
pub fn refresh(launch: &Git, selected: ReadyIssue, wait: TimeDelta) -> Result<Option<ReadyIssue>> {
    let reads = GitHubAndOrigin {
        launch,
        github: GitHub::new(),
    };
    let issue = selected.listed.issue;
    let current = reads.github.issue(&issue)?;
    if !current.is_open || !current.labels.has(READY_FOR_AGENT) {
        return Ok(None);
    }
    let listed = ListedIssue {
        issue,
        title: current.title.unwrap_or_default(),
        labels: current.labels,
    };
    match standing_of(&reads, &listed, Utc::now(), wait, Settling::Wait)? {
        Standing::Ready {
            is_spec,
            claim_generation,
        } if claim_generation == selected.claim_generation => Ok(Some(ReadyIssue {
            listed,
            is_spec,
            claim_generation,
        })),
        _ => Ok(None),
    }
}

/// What the search reads of an issue beyond its listing, each only when a
/// rule needs it.
trait Reads {
    /// `issue` as the search reads it: its parent, its sub-issues, its
    /// blockers and when it was last shaped.
    fn candidate(&self, issue: &IssueUrl) -> Result<Candidate>;
    /// What shows `issue` was ever started, if it was.
    fn started(&self, issue: &IssueUrl) -> Result<Option<Started>>;
    /// The Tickets of `spec`, an issue with sub-issues, as a Spec run reads
    /// them.
    fn tickets(&self, spec: &IssueUrl) -> Result<Vec<Ticket>>;
    fn issue_trust(&self, issue: &IssueUrl) -> Result<Option<UntrustedIssue>>;
}

/// The reads of GitHub and of the Launch directory `launch`'s origin.
struct GitHubAndOrigin<'a> {
    launch: &'a Git,
    github: GitHub,
}

impl Reads for GitHubAndOrigin<'_> {
    fn issue_trust(&self, issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
        trust::issue_trust(&self.github, issue)
    }

    fn candidate(&self, issue: &IssueUrl) -> Result<Candidate> {
        self.github.candidate(issue, READY_FOR_AGENT)
    }

    fn started(&self, issue: &IssueUrl) -> Result<Option<Started>> {
        branch::started(self.launch, &self.github, issue)
    }

    fn tickets(&self, spec: &IssueUrl) -> Result<Vec<Ticket>> {
        self.github.tickets(spec)
    }
}

/// Where an open issue labelled `ready-for-agent` stands in the search.
#[derive(Clone)]
enum Standing {
    /// It is a Ready issue.
    Ready {
        /// Whether it is a Spec, an issue with sub-issues.
        is_spec: bool,
        claim_generation: usize,
    },
    /// It is not one, for this reason: the first that applies.
    PassedOver(Reason),
}

/// Why an open issue labelled `ready-for-agent` is not a Ready issue.
#[derive(Clone)]
enum Reason {
    Untrusted(UntrustedIssue),
    /// It has this label, which makes an Unready Ticket.
    Unready(Label),
    /// It carries a Claim.
    Claimed,
    /// It is a Ticket of this Spec, which it is reached through.
    Ticket(IssueUrl),
    /// It is a Base fix issue, which the Run that opened it owns.
    BaseFix,
    /// It belongs to a Security run, even if Claim creation failed.
    SecurityFix,
    /// It is blocked by these open issues.
    Blocked(Vec<u64>),
    /// It was started, as this shows, without a recorded hand-back.
    Started(Started),
    /// It is a Spec whose Tickets are all closed: the Spec run it would be
    /// dispatched as refuses it, having nothing to do.
    TicketsClosed,
    /// It is a Spec with no startable Ticket, its Tickets as read: the Spec
    /// run it would be dispatched as could start none, and would fail.
    NoStartableTicket(Vec<Ticket>),
    /// It is not settled: it was last shaped, by this, within the
    /// configured settling window.
    Unsettled(Shaping),
}

/// The search of a repository's open issues labelled `ready-for-agent` for
/// the Ready issue at `order`'s end of the issue numbers.
struct Search<'a, R, P> {
    /// What it reads of each issue beyond its listing.
    reads: &'a R,
    /// The issues, in the order they are considered.
    candidates: Vec<ListedIssue>,
    /// Where each stands, in the same order, once it has been worked out.
    standings: Vec<Option<Standing>>,
    /// The time of the pass, which settling is measured to.
    now: DateTime<Utc>,
    /// How long an issue is left after it was last shaped before it is a
    /// Ready issue, so a Spec is not taken while Tickets are attached.
    wait: TimeDelta,
    /// Whether the search is also for an issue that is only settling.
    settling: Settling,
    /// Where each line on an issue passed over goes, as soon as it is.
    passed_over: P,
}

impl<'a, R: Reads, P: FnMut(String)> Search<'a, R, P> {
    fn new(
        reads: &'a R,
        mut candidates: Vec<ListedIssue>,
        now: DateTime<Utc>,
        wait: TimeDelta,
        order: PickupOrder,
        passed_over: P,
    ) -> Self {
        candidates.sort_by_key(|candidate| candidate.issue.number);
        if order == PickupOrder::HighestFirst {
            candidates.reverse();
        }
        let standings = vec![None; candidates.len()];
        Search {
            reads,
            candidates,
            standings,
            now,
            wait,
            settling: Settling::Wait,
            passed_over,
        }
    }

    fn first_priority(mut self, settling: Settling) -> Result<Option<PriorityIssue>> {
        self.settling = settling;
        for at in 0..self.candidates.len() {
            match self.standing(at)? {
                Standing::Ready {
                    is_spec,
                    claim_generation,
                } => {
                    let listed = self.candidates.swap_remove(at);
                    return Ok(Some(PriorityIssue::Ready(ReadyIssue {
                        listed,
                        is_spec,
                        claim_generation,
                    })));
                }
                Standing::PassedOver(Reason::Unsettled(_)) if settling == Settling::Include => {
                    return Ok(Some(PriorityIssue::Settling(
                        self.candidates.swap_remove(at),
                    )));
                }
                Standing::PassedOver(reason) => {
                    if let Some(line) = self.line(at, &reason)? {
                        (self.passed_over)(line);
                    }
                }
            }
        }
        Ok(None)
    }

    /// Where the candidate at `at` stands, worked out once.
    fn standing(&mut self, at: usize) -> Result<Standing> {
        if let Some(standing) = &self.standings[at] {
            return Ok(standing.clone());
        }
        let standing = standing_of(
            self.reads,
            &self.candidates[at],
            self.now,
            self.wait,
            self.settling,
        )?;
        self.standings[at] = Some(standing.clone());
        Ok(standing)
    }

    /// The line on the candidate at `at`, passed over for `reason`. A Ticket
    /// whose Spec is a Ready issue gets none: it is reached through its Spec,
    /// which a Pickup run takes.
    fn line(&mut self, at: usize, reason: &Reason) -> Result<Option<String>> {
        let number = self.candidates[at].issue.number;
        let why = match reason {
            Reason::Untrusted(check) => check.to_string(),
            Reason::Unready(label) => format!("labelled {label}"),
            Reason::Claimed => format!("labelled {}", claim::IN_PROGRESS),
            Reason::SecurityFix => "a Security fix: its Security run owns it".to_string(),
            Reason::Ticket(spec) => {
                let listed = self.candidates.iter().position(|candidate| {
                    candidate.issue.number == spec.number && candidate.issue.in_same_repo(spec)
                });
                let spec_is_ready = match listed {
                    Some(spec) => matches!(self.standing(spec)?, Standing::Ready { .. }),
                    None => false,
                };
                if spec_is_ready {
                    return Ok(None);
                }
                let spec = if self.candidates[at].issue.in_same_repo(spec) {
                    format!("#{}", spec.number)
                } else {
                    format!("{}#{}", spec.repo_slug(), spec.number)
                };
                format!("is a Ticket of {spec}, which is not ready")
            }
            Reason::BaseFix => format!("labelled {}", base_fix::BASE_FIX),
            Reason::Blocked(blockers) => {
                let blockers: Vec<String> = blockers
                    .iter()
                    .map(|blocker| format!("#{blocker}"))
                    .collect();
                format!("blocked by {}", blockers.join(", "))
            }
            Reason::Started(started) => {
                let evidence = match started {
                    Started::Branch(branch) => format!("{branch} is on origin"),
                    Started::PullRequest(url) => format!("PR {url}"),
                };
                format!(
                    "started: {evidence}; no ready-for-agent hand-back after the last Claim ended"
                )
            }
            Reason::TicketsClosed => "every Ticket is closed".to_string(),
            Reason::NoStartableTicket(tickets) => {
                let spec = &self.candidates[at].issue;
                format!(
                    "has no Ticket that can start: {}",
                    why_none_can_start(spec, tickets)
                )
            }
            Reason::Unsettled(shaping) => {
                let shaped = match shaping {
                    Shaping::Labelled => format!("labelled {READY_FOR_AGENT}"),
                    Shaping::SubIssues => "a sub-issue added or removed".to_string(),
                    Shaping::Blockers => "a \"blocked by\" link added or removed".to_string(),
                };
                let minutes = self.wait.num_minutes();
                format!("not settled: {shaped} less than {minutes} minutes ago")
            }
        };
        Ok(Some(format!("#{number} {why}")))
    }
}

/// Each open Ticket of `spec`, lowest number first, with why it can't start,
/// as in "#21 labelled needs-info; #22 blocked by #21".
fn why_none_can_start(spec: &IssueUrl, tickets: &[Ticket]) -> String {
    let name = |issue: &IssueUrl| {
        if spec.in_same_repo(issue) {
            format!("#{}", issue.number)
        } else {
            issue.url.clone()
        }
    };
    let in_spec = |issue: &IssueUrl| {
        tickets
            .iter()
            .any(|ticket| ticket.issue.identity() == issue.identity())
    };
    let mut by_number: Vec<&Ticket> = tickets.iter().collect();
    by_number.sort_by_key(|ticket| ticket.issue.number);
    let lines: Vec<String> = by_number
        .into_iter()
        .filter_map(|ticket| {
            let why = match spec_run::unstartable(spec, ticket)? {
                Unstartable::Closed => return None,
                Unstartable::OutsideRepo => "outside the Spec's repository".to_string(),
                Unstartable::Unready(label) => format!("labelled {label}"),
                Unstartable::HasSubIssues => "has sub-issues".to_string(),
                Unstartable::Blocked => {
                    let blockers: Vec<String> = ticket
                        .open_blockers
                        .iter()
                        .map(|blocker| {
                            let blocker = &blocker.issue;
                            if in_spec(blocker) {
                                name(blocker)
                            } else {
                                format!("{} (outside the Spec)", name(blocker))
                            }
                        })
                        .collect();
                    format!("blocked by {}", blockers.join(", "))
                }
            };
            Some(format!("{} {why}", name(&ticket.issue)))
        })
        .collect();
    lines.join("; ")
}

/// Where `candidate`, an open issue labelled `ready-for-agent`, stands at
/// `now`, reading what its listing doesn't give through `reads`. It is a
/// Ready issue when its author and latest ready-for-agent label actor are
/// Trusted accounts, it has no label that makes an Unready Ticket and no
/// Claim, is not a sub-issue, has no `base-fix` or `security-fix` label and no
/// open blocker, was never started or has a recorded ready-for-agent
/// hand-back after its last Claim ended, is not a Spec whose Tickets are all
/// closed, is settled: `wait` has passed since it was last labelled
/// `ready-for-agent` and since a sub-issue or a "blocked by" link of its was
/// last added or removed, and, if it is a Spec, has a startable Ticket. Its
/// Tickets are read only for that last check.
fn standing_of(
    reads: &impl Reads,
    candidate: &ListedIssue,
    now: DateTime<Utc>,
    wait: TimeDelta,
    settling: Settling,
) -> Result<Standing> {
    let passed_over = |reason| Ok(Standing::PassedOver(reason));
    if let Some(label) = candidate.labels.unready() {
        return passed_over(Reason::Unready(label));
    }
    if claim::is_on(&candidate.labels) {
        return passed_over(Reason::Claimed);
    }
    if candidate.labels.has(crate::security::fixing::SECURITY_FIX) {
        return passed_over(Reason::SecurityFix);
    }
    let read = reads.candidate(&candidate.issue)?;
    if let Some(spec) = read.parent {
        return passed_over(Reason::Ticket(spec));
    }
    if base_fix::is_issue(&candidate.labels) {
        return passed_over(Reason::BaseFix);
    }
    if !read.open_blockers.is_empty() {
        return passed_over(Reason::Blocked(read.open_blockers));
    }
    if let Some(check) = reads.issue_trust(&candidate.issue)? {
        return passed_over(Reason::Untrusted(check));
    }
    if let Some(started) = reads.started(&candidate.issue)?
        && !read.handed_back
    {
        return passed_over(Reason::Started(started));
    }
    if read.claimed {
        return passed_over(Reason::Claimed);
    }
    if spec_run::all_closed(read.sub_issue_is_open.iter().copied()) {
        return passed_over(Reason::TicketsClosed);
    }
    let unsettled = read
        .last_shaped
        .filter(|shaped| wait > TimeDelta::zero() && now - shaped.at < wait);
    // Pickup keeps its existing short circuit and passed-over reason.
    // Architect must check every other condition before treating settling
    // as the only reason an issue is not Ready.
    if settling == Settling::Wait
        && let Some(shaped) = &unsettled
    {
        return passed_over(Reason::Unsettled(shaped.by));
    }
    let is_spec = read.has_sub_issues();
    if is_spec {
        let tickets = reads.tickets(&candidate.issue)?;
        // Read after the candidate: its Tickets may have closed since.
        if !tickets.iter().any(|ticket| ticket.is_open) {
            return passed_over(Reason::TicketsClosed);
        }
        if tickets
            .iter()
            .all(|ticket| spec_run::unstartable(&candidate.issue, ticket).is_some())
        {
            return passed_over(Reason::NoStartableTicket(tickets));
        }
    }
    if let Some(shaped) = unsettled {
        return passed_over(Reason::Unsettled(shaped.by));
    }
    Ok(Standing::Ready {
        is_spec,
        claim_generation: read.claim_generation,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use chrono::TimeZone;

    use super::*;
    use crate::github::Shaped;

    /// What the search did, in the order it did it.
    #[derive(Debug, PartialEq, Eq)]
    enum Seen {
        /// It read this issue's candidate read.
        CandidateRead(u64),
        /// It checked whether this issue was started.
        StartedCheck(u64),
        /// It read this Spec's Tickets.
        TicketsRead(u64),
        /// It passed over an issue with this line.
        Line(String),
    }

    /// An issue's facts beyond its listing, as the in-memory reads give them.
    #[derive(Default)]
    struct Facts {
        /// The issue it is a sub-issue of, if it is one.
        parent: Option<IssueUrl>,
        /// For each of its sub-issues, whether that one is open.
        sub_issue_is_open: Vec<bool>,
        /// The numbers of the open issues it is blocked by.
        open_blockers: Vec<u64>,
        /// The latest of the changes that shape it, if there is one.
        last_shaped: Option<Shaped>,
        /// Label history establishes hand-back after the last Claim ended.
        handed_back: bool,
        /// What shows it was started, if it was.
        started: Option<Started>,
        /// Its Tickets as a Spec run reads them, if they are not each a
        /// startable Ticket, open or not as `sub_issue_is_open` says.
        tickets: Option<Vec<Ticket>>,
    }

    /// Reads from memory, each recorded in `seen`, with an issue that has no
    /// facts having none of them.
    struct InMemory<'a> {
        facts: HashMap<u64, Facts>,
        seen: &'a RefCell<Vec<Seen>>,
    }

    impl InMemory<'_> {
        /// The facts of `issue`, if it has any.
        fn facts(&self, issue: &IssueUrl) -> Option<&Facts> {
            self.facts.get(&issue.number)
        }
    }

    impl Reads for InMemory<'_> {
        fn issue_trust(&self, _issue: &IssueUrl) -> Result<Option<UntrustedIssue>> {
            Ok(None)
        }

        fn candidate(&self, issue: &IssueUrl) -> Result<Candidate> {
            self.seen
                .borrow_mut()
                .push(Seen::CandidateRead(issue.number));
            let facts = self.facts(issue);
            Ok(Candidate {
                claim_generation: 0,
                claimed: false,
                parent: facts.and_then(|facts| facts.parent.clone()),
                sub_issue_is_open: facts
                    .map(|facts| facts.sub_issue_is_open.clone())
                    .unwrap_or_default(),
                open_blockers: facts
                    .map(|facts| facts.open_blockers.clone())
                    .unwrap_or_default(),
                last_shaped: facts.and_then(|facts| facts.last_shaped),
                handed_back: facts.is_some_and(|facts| facts.handed_back),
            })
        }

        fn started(&self, issue: &IssueUrl) -> Result<Option<Started>> {
            self.seen
                .borrow_mut()
                .push(Seen::StartedCheck(issue.number));
            Ok(self.facts(issue).and_then(|facts| facts.started.clone()))
        }

        fn tickets(&self, spec: &IssueUrl) -> Result<Vec<Ticket>> {
            self.seen.borrow_mut().push(Seen::TicketsRead(spec.number));
            let facts = self.facts(spec).expect("only a Spec's Tickets are read");
            Ok(facts.tickets.clone().unwrap_or_else(|| {
                let numbers = 100..;
                numbers
                    .zip(&facts.sub_issue_is_open)
                    .map(|(number, &is_open)| Ticket {
                        is_open,
                        ..startable_ticket(number)
                    })
                    .collect()
            }))
        }
    }

    /// Ticket `number` of a Spec in `acme/widgets`: open, startable.
    fn startable_ticket(number: u64) -> Ticket {
        Ticket {
            issue: url(number),
            title: None,
            is_open: true,
            labels: [READY_FOR_AGENT.name()].into_iter().collect(),
            has_sub_issues: false,
            blockers: Vec::new(),
            open_blockers: Vec::new(),
        }
    }

    /// Ticket `number`, open and blocked by the open issues `blockers`.
    fn blocked_ticket(number: u64, blockers: &[IssueUrl]) -> Ticket {
        Ticket {
            blockers: blockers.to_vec(),
            open_blockers: blockers.iter().cloned().map(Into::into).collect(),
            ..startable_ticket(number)
        }
    }

    /// Ticket `number`, open and labelled `label` as well.
    fn labelled_ticket(number: u64, label: &str) -> Ticket {
        Ticket {
            labels: [READY_FOR_AGENT.name(), label].into_iter().collect(),
            ..startable_ticket(number)
        }
    }

    /// A Spec, settled and never started, with `tickets`.
    fn spec_with(tickets: Vec<Ticket>) -> Facts {
        Facts {
            sub_issue_is_open: tickets.iter().map(|ticket| ticket.is_open).collect(),
            tickets: Some(tickets),
            ..Facts::default()
        }
    }

    /// The URL of issue `number` in `acme/widgets`.
    fn url(number: u64) -> IssueUrl {
        IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}")).unwrap()
    }

    /// Issue `number` as the listing gives it, labelled `ready-for-agent`
    /// and `more`.
    fn listed(number: u64, more: &[&str]) -> ListedIssue {
        ListedIssue {
            issue: url(number),
            title: format!("Issue {number}"),
            labels: [READY_FOR_AGENT.name()]
                .iter()
                .chain(more)
                .copied()
                .collect(),
        }
    }

    /// The time of every pass.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap()
    }

    /// Shaped by `by`, `ago` before [`now`].
    fn shaped(by: Shaping, ago: TimeDelta) -> Option<Shaped> {
        Some(Shaped {
            by,
            at: now() - ago,
        })
    }

    /// A search of `candidates` with `facts`: the number of the Ready issue
    /// it found and whether it is a Spec, if it found one, and what it did.
    fn search(
        candidates: Vec<ListedIssue>,
        facts: Vec<(u64, Facts)>,
    ) -> (Option<(u64, bool)>, Vec<Seen>) {
        search_ordered(candidates, facts, PickupOrder::LowestFirst)
    }

    /// [`search`], taking its Ready issue from `order`'s end of the issue
    /// numbers.
    fn search_ordered(
        candidates: Vec<ListedIssue>,
        facts: Vec<(u64, Facts)>,
        order: PickupOrder,
    ) -> (Option<(u64, bool)>, Vec<Seen>) {
        let seen = RefCell::new(Vec::new());
        let reads = InMemory {
            facts: facts.into_iter().collect(),
            seen: &seen,
        };
        let sink = |line| seen.borrow_mut().push(Seen::Line(line));
        let ready = Search::new(
            &reads,
            candidates,
            now(),
            TimeDelta::minutes(30),
            order,
            sink,
        )
        .first_priority(Settling::Wait)
        .unwrap()
        .map(|priority| {
            let PriorityIssue::Ready(ready) = priority else {
                panic!("a search waiting for settling selected an unsettled issue");
            };
            (ready.listed.issue.number, ready.is_spec)
        });
        (ready, seen.into_inner())
    }

    /// What the search did on passing over an issue with `line`.
    fn passed_over(line: &str) -> Seen {
        Seen::Line(line.to_string())
    }

    /// Every read of issue `number` the search makes for a Ready issue.
    fn every_read(number: u64) -> [Seen; 2] {
        [Seen::CandidateRead(number), Seen::StartedCheck(number)]
    }

    #[test]
    fn an_issue_with_a_label_that_makes_an_unready_ticket_is_passed_over_with_no_reads() {
        for (label, named) in [
            ("ready-for-human", "ready-for-human"),
            ("needs-info", "needs-info"),
            ("wontfix", "wontfix"),
            ("needs-triage", "needs-triage"),
            ("Needs-Info", "needs-info"),
        ] {
            let (ready, seen) = search(vec![listed(7, &[label])], vec![]);

            assert_eq!(ready, None, "{label}");
            assert_eq!(
                seen,
                [passed_over(&format!("#7 labelled {named}"))],
                "{label}"
            );
        }
    }

    #[test]
    fn a_claimed_issue_is_passed_over_with_no_reads() {
        let (ready, seen) = search(vec![listed(7, &["in-progress"])], vec![]);

        assert_eq!(ready, None);
        assert_eq!(seen, [passed_over("#7 labelled in-progress")]);
    }

    #[test]
    fn a_ticket_whose_spec_is_not_listed_names_it_with_no_started_check() {
        let ticket = Facts {
            parent: Some(url(3)),
            ..Facts::default()
        };
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, ticket)]);

        assert_eq!(ready, None);
        assert_eq!(
            seen,
            [
                Seen::CandidateRead(7),
                passed_over("#7 is a Ticket of #3, which is not ready"),
            ]
        );
    }

    #[test]
    fn a_ticket_of_a_spec_in_another_repository_names_it_with_its_repository() {
        let spec = IssueUrl::parse("https://github.com/acme/gadgets/issues/7").unwrap();
        let ticket = Facts {
            parent: Some(spec),
            ..Facts::default()
        };
        let (_, seen) = search(vec![listed(7, &[])], vec![(7, ticket)]);

        assert_eq!(
            seen,
            [
                Seen::CandidateRead(7),
                passed_over("#7 is a Ticket of acme/gadgets#7, which is not ready"),
            ]
        );
    }

    #[test]
    fn a_ticket_whose_spec_is_listed_but_not_ready_names_it_above_and_below_the_spec() {
        // #7 is a Ticket below the Spec #20, #21 one above it, and #20 is
        // blocked.
        let ticket = || Facts {
            parent: Some(url(20)),
            ..Facts::default()
        };
        let spec = Facts {
            sub_issue_is_open: vec![true, true],
            open_blockers: vec![30],
            ..Facts::default()
        };
        let (ready, seen) = search(
            vec![listed(21, &[]), listed(20, &[]), listed(7, &[])],
            vec![(7, ticket()), (20, spec), (21, ticket())],
        );

        assert_eq!(ready, None);
        // The Spec is read once, for #7's line, and not again for its own.
        assert_eq!(
            seen,
            [
                Seen::CandidateRead(7),
                Seen::CandidateRead(20),
                passed_over("#7 is a Ticket of #20, which is not ready"),
                passed_over("#20 blocked by #30"),
                Seen::CandidateRead(21),
                passed_over("#21 is a Ticket of #20, which is not ready"),
            ]
        );
    }

    #[test]
    fn a_ticket_whose_spec_is_a_ready_issue_gets_no_line_above_or_below_the_spec() {
        let ticket = || Facts {
            parent: Some(url(20)),
            ..Facts::default()
        };
        let spec = || Facts {
            sub_issue_is_open: vec![true, true],
            ..Facts::default()
        };

        // Below the Spec: passed over with no line, and the Spec taken.
        let (ready, seen) = search(
            vec![listed(7, &[]), listed(20, &[]), listed(21, &[])],
            vec![(7, ticket()), (20, spec()), (21, ticket())],
        );
        assert_eq!(ready, Some((20, true)));
        let [candidate_20, started_20] = every_read(20);
        assert_eq!(
            seen,
            [
                Seen::CandidateRead(7),
                candidate_20,
                started_20,
                Seen::TicketsRead(20)
            ],
            "below"
        );

        // Above the Spec: never reached, the Spec being taken first.
        let (ready, seen) = search(
            vec![listed(20, &[]), listed(21, &[])],
            vec![(20, spec()), (21, ticket())],
        );
        assert_eq!(ready, Some((20, true)));
        let [candidate_20, started_20] = every_read(20);
        assert_eq!(
            seen,
            [candidate_20, started_20, Seen::TicketsRead(20)],
            "above"
        );
    }

    #[test]
    fn a_base_fix_issue_is_passed_over_with_no_started_check() {
        for label in ["base-fix", "Base-Fix"] {
            let (ready, seen) = search(vec![listed(7, &[label])], vec![]);

            assert_eq!(ready, None, "{label}");
            assert_eq!(
                seen,
                [Seen::CandidateRead(7), passed_over("#7 labelled base-fix")],
                "{label}"
            );
        }
    }

    #[test]
    fn a_blocked_issue_names_every_open_blocker_with_no_started_check() {
        let blocked = Facts {
            open_blockers: vec![5, 6],
            ..Facts::default()
        };
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, blocked)]);

        assert_eq!(ready, None);
        assert_eq!(
            seen,
            [Seen::CandidateRead(7), passed_over("#7 blocked by #5, #6")]
        );
    }

    #[test]
    fn a_never_claimed_started_issue_explains_the_missing_hand_back() {
        let pr = "https://github.com/acme/widgets/pull/12";
        for (started, said) in [
            (
                Started::Branch("issue-7-branch-2".to_string()),
                "#7 started: issue-7-branch-2 is on origin; no ready-for-agent hand-back after the last Claim ended".to_string(),
            ),
            (
                Started::PullRequest(pr.to_string()),
                format!("#7 started: PR {pr}; no ready-for-agent hand-back after the last Claim ended"),
            ),
        ] {
            let facts = Facts {
                started: Some(started),
                ..Facts::default()
            };
            let (ready, seen) = search(vec![listed(7, &[])], vec![(7, facts)]);

            assert_eq!(ready, None);
            let [candidate, started] = every_read(7);
            assert_eq!(seen, [candidate, started, passed_over(&said)]);
        }
    }

    #[test]
    fn a_handed_back_issue_with_a_branch_or_open_pr_is_ready() {
        for started in [
            Started::Branch("issue-7".to_string()),
            Started::PullRequest("https://github.com/acme/widgets/pull/12".to_string()),
        ] {
            let facts = Facts {
                started: Some(started),
                handed_back: true,
                last_shaped: shaped(Shaping::Labelled, TimeDelta::minutes(31)),
                ..Facts::default()
            };
            let (ready, _) = search(vec![listed(7, &[])], vec![(7, facts)]);
            assert_eq!(ready, Some((7, false)));
        }
    }

    #[test]
    fn hand_back_preserves_every_other_readiness_gate_and_the_relabel_settling_window() {
        for (labels, parent, blockers, tickets, recent, reason) in [
            (
                vec!["needs-triage"],
                None,
                vec![],
                vec![],
                false,
                "labelled needs-triage",
            ),
            (
                vec!["in-progress"],
                None,
                vec![],
                vec![],
                false,
                "labelled in-progress",
            ),
            (
                vec!["base-fix"],
                None,
                vec![],
                vec![],
                false,
                "labelled base-fix",
            ),
            (
                vec!["security-fix"],
                None,
                vec![],
                vec![],
                false,
                "a Security fix: its Security run owns it",
            ),
            (
                vec![],
                Some(url(3)),
                vec![],
                vec![],
                false,
                "is a Ticket of #3, which is not ready",
            ),
            (vec![], None, vec![9], vec![], false, "blocked by #9"),
            (
                vec![],
                None,
                vec![],
                vec![false],
                false,
                "every Ticket is closed",
            ),
            (
                vec![],
                None,
                vec![],
                vec![],
                true,
                "not settled: labelled ready-for-agent less than 30 minutes ago",
            ),
        ] {
            let facts = Facts {
                started: Some(Started::Branch("issue-7".to_string())),
                handed_back: true,
                parent,
                open_blockers: blockers,
                sub_issue_is_open: tickets,
                last_shaped: shaped(
                    Shaping::Labelled,
                    TimeDelta::minutes(if recent { 29 } else { 31 }),
                ),
                tickets: None,
            };
            let (ready, seen) = search(vec![listed(7, &labels)], vec![(7, facts)]);
            assert_eq!(ready, None, "{reason}");
            assert_eq!(seen.last(), Some(&passed_over(&format!("#7 {reason}"))));
        }
    }

    #[test]
    fn a_spec_whose_tickets_are_all_closed_is_passed_over_unless_it_was_started() {
        let spec = |started: Option<Started>| Facts {
            sub_issue_is_open: vec![false, false],
            started,
            ..Facts::default()
        };

        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec(None))]);
        assert_eq!(ready, None);
        let [candidate, started] = every_read(7);
        assert_eq!(
            seen,
            [candidate, started, passed_over("#7 every Ticket is closed")]
        );

        let branch = Started::Branch("issue-7".to_string());
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec(Some(branch)))]);
        assert_eq!(ready, None);
        assert_eq!(
            seen.last(),
            Some(&passed_over(
                "#7 started: issue-7 is on origin; no ready-for-agent hand-back after the last Claim ended"
            ))
        );
    }

    #[test]
    fn a_spec_with_a_ticket_still_open_is_a_ready_issue_and_a_spec() {
        let spec = Facts {
            sub_issue_is_open: vec![false, true],
            ..Facts::default()
        };
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);

        assert_eq!(ready, Some((7, true)));
        let [candidate, started] = every_read(7);
        assert_eq!(seen, [candidate, started, Seen::TicketsRead(7)]);
    }

    #[test]
    fn a_spec_whose_only_open_ticket_is_unready_is_passed_over_naming_the_ticket_and_label() {
        let mut done = startable_ticket(8);
        done.is_open = false;
        let spec = spec_with(vec![done, labelled_ticket(9, "needs-info")]);

        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);

        assert_eq!(ready, None);
        let [candidate, started] = every_read(7);
        assert_eq!(
            seen,
            [
                candidate,
                started,
                Seen::TicketsRead(7),
                passed_over("#7 has no Ticket that can start: #9 labelled needs-info"),
            ]
        );
    }

    #[test]
    fn a_spec_with_no_startable_ticket_names_why_each_open_ticket_cant_start() {
        let foreign = IssueUrl::parse("https://github.com/other/widgets/issues/21").unwrap();
        let mut parent = startable_ticket(12);
        parent.has_sub_issues = true;
        let spec = spec_with(vec![
            blocked_ticket(11, &[url(30)]),
            blocked_ticket(10, &[url(9)]),
            labelled_ticket(9, "needs-triage"),
            parent,
            Ticket {
                issue: foreign,
                ..startable_ticket(21)
            },
        ]);

        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);

        assert_eq!(ready, None);
        assert_eq!(
            seen.last(),
            Some(&passed_over(
                "#7 has no Ticket that can start: #9 labelled needs-triage; \
                 #10 blocked by #9; #11 blocked by #30 (outside the Spec); \
                 #12 has sub-issues; \
                 https://github.com/other/widgets/issues/21 outside the Spec's repository"
            ))
        );
    }

    #[test]
    fn a_spec_whose_tickets_closed_since_the_candidate_read_says_every_ticket_is_closed() {
        let mut closed = startable_ticket(8);
        closed.is_open = false;
        let spec = Facts {
            sub_issue_is_open: vec![true],
            ..spec_with(vec![closed])
        };
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);
        assert_eq!(ready, None);
        assert_eq!(seen.last(), Some(&passed_over("#7 every Ticket is closed")));
    }

    #[test]
    fn a_spec_whose_tickets_read_empty_says_every_ticket_is_closed() {
        let spec = Facts {
            sub_issue_is_open: vec![true],
            tickets: Some(vec![]),
            ..Facts::default()
        };
        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);
        assert_eq!(ready, None);
        assert_eq!(seen.last(), Some(&passed_over("#7 every Ticket is closed")));
    }

    #[test]
    fn a_spec_with_one_startable_ticket_among_unready_ones_is_a_ready_issue() {
        let spec = spec_with(vec![
            labelled_ticket(8, "ready-for-human"),
            startable_ticket(9),
        ]);

        let (ready, _) = search(vec![listed(7, &[])], vec![(7, spec)]);

        assert_eq!(ready, Some((7, true)));
    }

    #[test]
    fn a_handed_back_spec_with_no_startable_ticket_is_passed_over() {
        let spec = Facts {
            started: Some(Started::Branch("issue-7".to_string())),
            handed_back: true,
            ..spec_with(vec![labelled_ticket(8, "wontfix")])
        };

        let (ready, seen) = search(vec![listed(7, &[])], vec![(7, spec)]);

        assert_eq!(ready, None);
        assert_eq!(
            seen.last(),
            Some(&passed_over(
                "#7 has no Ticket that can start: #8 labelled wontfix"
            ))
        );
    }

    #[test]
    fn a_spec_failing_an_earlier_gate_has_its_tickets_never_read() {
        let mut closed = startable_ticket(8);
        closed.is_open = false;
        let unsettled = Facts {
            last_shaped: shaped(Shaping::SubIssues, TimeDelta::minutes(5)),
            ..spec_with(vec![labelled_ticket(9, "needs-info")])
        };
        let blocked = Facts {
            open_blockers: vec![30],
            ..spec_with(vec![labelled_ticket(9, "needs-info")])
        };

        let (ready, seen) = search(
            vec![listed(5, &[]), listed(6, &[]), listed(7, &[])],
            vec![(5, spec_with(vec![closed])), (6, unsettled), (7, blocked)],
        );

        assert_eq!(ready, None);
        assert!(
            !seen.iter().any(|seen| matches!(seen, Seen::TicketsRead(_))),
            "{seen:?}"
        );
        let lines: Vec<&Seen> = seen
            .iter()
            .filter(|seen| matches!(seen, Seen::Line(_)))
            .collect();
        assert_eq!(
            lines,
            [
                &passed_over("#5 every Ticket is closed"),
                &passed_over(
                    "#6 not settled: a sub-issue added or removed less than 30 minutes ago"
                ),
                &passed_over("#7 blocked by #30"),
            ]
        );
    }

    #[test]
    fn an_issue_with_no_sub_issues_is_a_ready_issue_and_not_a_spec() {
        let (ready, seen) = search(vec![listed(7, &[])], vec![]);

        assert_eq!(ready, Some((7, false)));
        assert_eq!(seen, every_read(7));
    }

    #[test]
    fn an_issue_settles_thirty_minutes_after_it_was_last_shaped_whatever_shaped_it() {
        let wait = TimeDelta::minutes(30);
        let just_under = wait - TimeDelta::seconds(1);
        let just_over = wait + TimeDelta::seconds(1);
        for (by, said) in [
            (Shaping::Labelled, "labelled ready-for-agent"),
            (Shaping::SubIssues, "a sub-issue added or removed"),
            (Shaping::Blockers, "a \"blocked by\" link added or removed"),
        ] {
            for (ago, settled) in [(just_under, false), (wait, true), (just_over, true)] {
                let facts = Facts {
                    last_shaped: shaped(by, ago),
                    ..Facts::default()
                };
                let (ready, seen) = search(vec![listed(7, &[])], vec![(7, facts)]);

                let [candidate, started] = every_read(7);
                if settled {
                    assert_eq!(ready, Some((7, false)), "{said}, {ago}");
                    assert_eq!(seen, [candidate, started], "{said}, {ago}");
                } else {
                    assert_eq!(ready, None, "{said}, {ago}");
                    let line =
                        passed_over(&format!("#7 not settled: {said} less than 30 minutes ago"));
                    assert_eq!(seen, [candidate, started, line], "{said}, {ago}");
                }
            }
        }
    }

    #[test]
    fn a_passed_over_issue_has_one_line_with_the_first_reason_that_applies() {
        // Every reason applies to #7 in the first row: each row takes the
        // first one away. Tickets all closed and not settled can't both
        // apply with nothing started, so not settled has a row of its own,
        // and no startable Ticket the last, once #7 has settled.
        let facts = |parent: bool, blocked: bool, started: bool, closed: bool| Facts {
            parent: parent.then(|| url(3)),
            sub_issue_is_open: vec![!closed],
            open_blockers: if blocked { vec![5] } else { vec![] },
            last_shaped: shaped(Shaping::Labelled, TimeDelta::minutes(1)),
            started: started.then(|| Started::Branch("issue-7".to_string())),
            handed_back: false,
            tickets: Some(vec![Ticket {
                is_open: !closed,
                ..labelled_ticket(8, "needs-info")
            }]),
        };
        let settled = Facts {
            last_shaped: None,
            ..facts(false, false, false, false)
        };
        for (labels, facts, said) in [
            (
                &["needs-info", "in-progress", "base-fix"][..],
                facts(true, true, true, true),
                "#7 labelled needs-info",
            ),
            (
                &["in-progress", "base-fix"],
                facts(true, true, true, true),
                "#7 labelled in-progress",
            ),
            (
                &["base-fix"],
                facts(true, true, true, true),
                "#7 is a Ticket of #3, which is not ready",
            ),
            (
                &["base-fix"],
                facts(false, true, true, true),
                "#7 labelled base-fix",
            ),
            (&[], facts(false, true, true, true), "#7 blocked by #5"),
            (
                &[],
                facts(false, false, true, true),
                "#7 started: issue-7 is on origin; no ready-for-agent hand-back after the last Claim ended",
            ),
            (
                &[],
                facts(false, false, false, true),
                "#7 every Ticket is closed",
            ),
            (
                &[],
                facts(false, false, false, false),
                "#7 not settled: labelled ready-for-agent less than 30 minutes ago",
            ),
            (
                &[],
                settled,
                "#7 has no Ticket that can start: #8 labelled needs-info",
            ),
        ] {
            let (ready, seen) = search(vec![listed(7, labels)], vec![(7, facts)]);

            assert_eq!(ready, None, "{said}");
            assert_eq!(seen.last(), Some(&passed_over(said)));
            let lines = seen.iter().filter(|seen| matches!(seen, Seen::Line(_)));
            assert_eq!(lines.count(), 1, "{said}");
        }
    }

    #[test]
    fn each_line_comes_as_its_issue_is_passed_over_and_the_search_stops_at_the_first_ready_issue() {
        let started = Facts {
            started: Some(Started::Branch("issue-7".to_string())),
            ..Facts::default()
        };
        let (ready, seen) = search(
            vec![
                listed(71, &["needs-info"]),
                listed(70, &[]),
                listed(9, &["in-progress"]),
                listed(8, &["wontfix"]),
                listed(7, &[]),
            ],
            vec![(7, started)],
        );

        assert_eq!(ready, Some((70, false)));
        let [candidate_7, started_7] = every_read(7);
        let [candidate_70, started_70] = every_read(70);
        assert_eq!(
            seen,
            [
                candidate_7,
                started_7,
                passed_over(
                    "#7 started: issue-7 is on origin; no ready-for-agent hand-back after the last Claim ended"
                ),
                passed_over("#8 labelled wontfix"),
                passed_over("#9 labelled in-progress"),
                candidate_70,
                started_70,
            ]
        );
    }

    #[test]
    fn pickup_order_names_parse_back_to_themselves() {
        for order in [PickupOrder::LowestFirst, PickupOrder::HighestFirst] {
            assert_eq!(PickupOrder::parse(order.name()), Some(order));
        }
        assert_eq!(PickupOrder::parse("lowest"), None);
        assert_eq!(PickupOrder::parse(""), None);
        assert_eq!(PickupOrder::default(), PickupOrder::LowestFirst);
    }

    #[test]
    fn highest_first_takes_the_highest_ready_issue_passing_over_downward() {
        let started = Facts {
            started: Some(Started::Branch("issue-71".to_string())),
            ..Facts::default()
        };
        let (ready, seen) = search_ordered(
            vec![
                listed(7, &[]),
                listed(9, &["in-progress"]),
                listed(70, &[]),
                listed(71, &[]),
            ],
            vec![(71, started)],
            PickupOrder::HighestFirst,
        );

        assert_eq!(ready, Some((70, false)));
        let [candidate_71, started_71] = every_read(71);
        let [candidate_70, started_70] = every_read(70);
        assert_eq!(
            seen,
            [
                candidate_71,
                started_71,
                passed_over(
                    "#71 started: issue-71 is on origin; no ready-for-agent hand-back after the last Claim ended"
                ),
                candidate_70,
                started_70,
            ]
        );
    }

    #[test]
    fn highest_first_gives_no_line_to_a_ticket_above_its_ready_spec() {
        let ticket = || Facts {
            parent: Some(url(20)),
            ..Facts::default()
        };
        let spec = || Facts {
            sub_issue_is_open: vec![true, true],
            ..Facts::default()
        };
        let (ready, seen) = search_ordered(
            vec![listed(7, &[]), listed(20, &[]), listed(21, &[])],
            vec![(7, ticket()), (20, spec()), (21, ticket())],
            PickupOrder::HighestFirst,
        );

        assert_eq!(ready, Some((20, true)));
        let [candidate_20, started_20] = every_read(20);
        assert_eq!(
            seen,
            [
                Seen::CandidateRead(21),
                candidate_20,
                started_20,
                Seen::TicketsRead(20),
            ]
        );
    }
}
