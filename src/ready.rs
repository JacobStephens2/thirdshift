//! The search for the lowest-numbered Ready issue in a repository, saying
//! why it passed over each issue labelled `ready-for-agent` before it. A
//! Pickup run takes the issue it finds; any run can ask whether a repository
//! has one, against the very same definition.

use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};

use crate::base_fix;
use crate::branch::{self, Started};
use crate::claim;
use crate::git::Git;
use crate::github::{Candidate, GitHub, ListedIssue, Shaping};
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Label, READY_FOR_AGENT};
use crate::progress;
use crate::spec_run;

/// The lowest-numbered Ready issue a search found.
pub struct ReadyIssue {
    /// The issue, as the search listed it.
    pub listed: ListedIssue,
    /// Whether it is a Spec, an issue with sub-issues.
    pub is_spec: bool,
}

/// The lowest-numbered Ready issue in `repo`, the Launch directory
/// `launch`'s repository, if there is one, with a line on stderr for each
/// issue labelled `ready-for-agent` passed over before it, with why.
/// Nothing is changed, on GitHub or in `launch`.
/// `wait` is the User config's settling window since the latest shaping
/// event; every pass uses the same Ready issue definition.
pub fn first(launch: &Git, repo: &Repo, wait: TimeDelta) -> Result<Option<ReadyIssue>> {
    let reads = GitHubAndOrigin {
        launch,
        github: GitHub::new(),
    };
    let candidates = reads
        .github
        .open_issues_labelled(&repo.slug(), READY_FOR_AGENT)?;
    Search::new(&reads, candidates, Utc::now(), wait, progress::step).first_ready()
}

/// What the search reads of an issue beyond its listing, each only when a
/// rule needs it.
trait Reads {
    /// `issue` as the search reads it: its parent, its sub-issues, its
    /// blockers and when it was last shaped.
    fn candidate(&self, issue: &IssueUrl) -> Result<Candidate>;
    /// What shows `issue` was ever started, if it was.
    fn started(&self, issue: &IssueUrl) -> Result<Option<Started>>;
}

/// The reads of GitHub and of the Launch directory `launch`'s origin.
struct GitHubAndOrigin<'a> {
    launch: &'a Git,
    github: GitHub,
}

impl Reads for GitHubAndOrigin<'_> {
    fn candidate(&self, issue: &IssueUrl) -> Result<Candidate> {
        self.github.candidate(issue, READY_FOR_AGENT)
    }

    fn started(&self, issue: &IssueUrl) -> Result<Option<Started>> {
        branch::started(self.launch, &self.github, issue)
    }
}

/// Where an open issue labelled `ready-for-agent` stands in the search.
#[derive(Clone)]
enum Standing {
    /// It is a Ready issue.
    Ready {
        /// Whether it is a Spec, an issue with sub-issues.
        is_spec: bool,
    },
    /// It is not one, for this reason: the first that applies.
    PassedOver(Reason),
}

/// Why an open issue labelled `ready-for-agent` is not a Ready issue.
#[derive(Clone)]
enum Reason {
    /// It has this label, which makes an Unready Ticket.
    Unready(Label),
    /// It carries a Claim.
    Claimed,
    /// It is a Ticket of this Spec, which it is reached through.
    Ticket(IssueUrl),
    /// It is a Base fix issue, which the Run that opened it owns.
    BaseFix,
    /// It is blocked by these open issues.
    Blocked(Vec<u64>),
    /// It was started, as this shows.
    Started(Started),
    /// It is a Spec whose Tickets are all closed, with nothing started: the
    /// Spec run it would be dispatched as refuses it, having nothing to do.
    TicketsClosed,
    /// It is not settled: it was last shaped, by this, within the
    /// configured settling window.
    Unsettled(Shaping),
}

/// The search of a repository's open issues labelled `ready-for-agent` for
/// the lowest-numbered Ready issue.
struct Search<'a, R, P> {
    /// What it reads of each issue beyond its listing.
    reads: &'a R,
    /// The issues, lowest number first.
    candidates: Vec<ListedIssue>,
    /// Where each stands, in the same order, once it has been worked out.
    standings: Vec<Option<Standing>>,
    /// The time of the pass, which settling is measured to.
    now: DateTime<Utc>,
    /// How long an issue is left after it was last shaped before it is a
    /// Ready issue, so a Spec is not taken while Tickets are attached.
    wait: TimeDelta,
    /// Where each line on an issue passed over goes, as soon as it is.
    passed_over: P,
}

impl<'a, R: Reads, P: FnMut(String)> Search<'a, R, P> {
    fn new(
        reads: &'a R,
        mut candidates: Vec<ListedIssue>,
        now: DateTime<Utc>,
        wait: TimeDelta,
        passed_over: P,
    ) -> Self {
        candidates.sort_by_key(|candidate| candidate.issue.number);
        let standings = vec![None; candidates.len()];
        Search {
            reads,
            candidates,
            standings,
            now,
            wait,
            passed_over,
        }
    }

    /// The lowest-numbered Ready issue, if there is one, with a line for
    /// each issue passed over before it.
    fn first_ready(mut self) -> Result<Option<ReadyIssue>> {
        for at in 0..self.candidates.len() {
            match self.standing(at)? {
                Standing::Ready { is_spec } => {
                    let listed = self.candidates.swap_remove(at);
                    return Ok(Some(ReadyIssue { listed, is_spec }));
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
        let standing = standing_of(self.reads, &self.candidates[at], self.now, self.wait)?;
        self.standings[at] = Some(standing.clone());
        Ok(standing)
    }

    /// The line on the candidate at `at`, passed over for `reason`. A Ticket
    /// whose Spec is a Ready issue gets none: it is reached through its Spec,
    /// which a Pickup run takes.
    fn line(&mut self, at: usize, reason: &Reason) -> Result<Option<String>> {
        let number = self.candidates[at].issue.number;
        let why = match reason {
            Reason::Unready(label) => format!("labelled {label}"),
            Reason::Claimed => format!("labelled {}", claim::IN_PROGRESS),
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
            Reason::Started(Started::Branch(branch)) => {
                format!("already started: {branch} is on origin")
            }
            Reason::Started(Started::PullRequest(url)) => format!("already started: PR {url}"),
            Reason::TicketsClosed => "every Ticket is closed".to_string(),
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

/// Where `candidate`, an open issue labelled `ready-for-agent`, stands at
/// `now`, reading what its listing doesn't give through `reads`. It is a
/// Ready issue when it has no label that makes an Unready Ticket and no
/// Claim, is not a sub-issue, has no `base-fix` label and no open blocker,
/// was never started, is not a Spec whose Tickets are all closed, and is
/// settled: `wait` has passed since it was last labelled
/// `ready-for-agent` and since a sub-issue or a "blocked by" link of its was
/// last added or removed.
fn standing_of(
    reads: &impl Reads,
    candidate: &ListedIssue,
    now: DateTime<Utc>,
    wait: TimeDelta,
) -> Result<Standing> {
    let passed_over = |reason| Ok(Standing::PassedOver(reason));
    if let Some(label) = candidate.labels.unready() {
        return passed_over(Reason::Unready(label));
    }
    if claim::is_on(&candidate.labels) {
        return passed_over(Reason::Claimed);
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
    if let Some(started) = reads.started(&candidate.issue)? {
        return passed_over(Reason::Started(started));
    }
    if spec_run::all_closed(read.sub_issue_is_open.iter().copied()) {
        return passed_over(Reason::TicketsClosed);
    }
    if let Some(shaped) = read.last_shaped
        && now - shaped.at < wait
    {
        return passed_over(Reason::Unsettled(shaped.by));
    }
    Ok(Standing::Ready {
        is_spec: read.has_sub_issues(),
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
        /// What shows it was started, if it was.
        started: Option<Started>,
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
        fn candidate(&self, issue: &IssueUrl) -> Result<Candidate> {
            self.seen
                .borrow_mut()
                .push(Seen::CandidateRead(issue.number));
            let facts = self.facts(issue);
            Ok(Candidate {
                parent: facts.and_then(|facts| facts.parent.clone()),
                sub_issue_is_open: facts
                    .map(|facts| facts.sub_issue_is_open.clone())
                    .unwrap_or_default(),
                open_blockers: facts
                    .map(|facts| facts.open_blockers.clone())
                    .unwrap_or_default(),
                last_shaped: facts.and_then(|facts| facts.last_shaped),
            })
        }

        fn started(&self, issue: &IssueUrl) -> Result<Option<Started>> {
            self.seen
                .borrow_mut()
                .push(Seen::StartedCheck(issue.number));
            Ok(self.facts(issue).and_then(|facts| facts.started.clone()))
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
        let seen = RefCell::new(Vec::new());
        let reads = InMemory {
            facts: facts.into_iter().collect(),
            seen: &seen,
        };
        let sink = |line| seen.borrow_mut().push(Seen::Line(line));
        let ready = Search::new(&reads, candidates, now(), TimeDelta::minutes(30), sink)
            .first_ready()
            .unwrap()
            .map(|ready| (ready.listed.issue.number, ready.is_spec));
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
            [Seen::CandidateRead(7), candidate_20, started_20],
            "below"
        );

        // Above the Spec: never reached, the Spec being taken first.
        let (ready, seen) = search(
            vec![listed(20, &[]), listed(21, &[])],
            vec![(20, spec()), (21, ticket())],
        );
        assert_eq!(ready, Some((20, true)));
        assert_eq!(seen, every_read(20), "above");
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
    fn a_started_issue_says_what_shows_it_was_started() {
        let pr = "https://github.com/acme/widgets/pull/12";
        for (started, said) in [
            (
                Started::Branch("issue-7-branch-2".to_string()),
                "#7 already started: issue-7-branch-2 is on origin".to_string(),
            ),
            (
                Started::PullRequest(pr.to_string()),
                format!("#7 already started: PR {pr}"),
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
            Some(&passed_over("#7 already started: issue-7 is on origin"))
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
        assert_eq!(seen, every_read(7));
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
        // apply with nothing started, so not settled has a row of its own.
        let facts = |parent: bool, blocked: bool, started: bool, closed: bool| Facts {
            parent: parent.then(|| url(3)),
            sub_issue_is_open: if closed { vec![false] } else { vec![] },
            open_blockers: if blocked { vec![5] } else { vec![] },
            last_shaped: shaped(Shaping::Labelled, TimeDelta::minutes(1)),
            started: started.then(|| Started::Branch("issue-7".to_string())),
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
                "#7 already started: issue-7 is on origin",
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
                passed_over("#7 already started: issue-7 is on origin"),
                passed_over("#8 labelled wontfix"),
                passed_over("#9 labelled in-progress"),
                candidate_70,
                started_70,
            ]
        );
    }
}
