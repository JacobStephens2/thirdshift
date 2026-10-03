//! The search for the lowest-numbered Ready issue in a repository, saying
//! why it passed over each issue labelled `ready-for-agent` before it. A
//! Pickup run takes the issue it finds; any run can ask whether a repository
//! has one, against the very same definition.

use anyhow::Result;
use chrono::{TimeDelta, Utc};

use crate::base_fix;
use crate::branch::{self, Started};
use crate::claim;
use crate::git::Git;
use crate::github::{self, ListedIssue, Shaping};
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
pub fn first(launch: &Git, repo: &Repo) -> Result<Option<ReadyIssue>> {
    let candidates = github::open_issues_labelled(&repo.slug(), READY_FOR_AGENT)?;
    Search::new(launch, candidates).first_ready()
}

/// How long an issue is left after it was last shaped before it is a Ready
/// issue, so a Spec is not taken while its Tickets are still being
/// attached. Fixed, with no setting.
const SETTLE: TimeDelta = TimeDelta::minutes(10);

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
    /// It is not settled: it was last shaped, by this, less than [`SETTLE`]
    /// ago.
    Unsettled(Shaping),
}

/// The search of a repository's open issues labelled `ready-for-agent` for
/// the lowest-numbered Ready issue.
struct Search<'a> {
    /// The Launch directory.
    launch: &'a Git,
    /// The issues, lowest number first.
    candidates: Vec<ListedIssue>,
    /// Where each stands, in the same order, once it has been worked out.
    standings: Vec<Option<Standing>>,
}

impl<'a> Search<'a> {
    fn new(launch: &'a Git, mut candidates: Vec<ListedIssue>) -> Self {
        candidates.sort_by_key(|candidate| candidate.issue.number);
        let standings = vec![None; candidates.len()];
        Search {
            launch,
            candidates,
            standings,
        }
    }

    /// The lowest-numbered Ready issue, if there is one, with a line on
    /// stderr for each issue passed over before it.
    fn first_ready(mut self) -> Result<Option<ReadyIssue>> {
        for at in 0..self.candidates.len() {
            match self.standing(at)? {
                Standing::Ready { is_spec } => {
                    let listed = self.candidates.swap_remove(at);
                    return Ok(Some(ReadyIssue { listed, is_spec }));
                }
                Standing::PassedOver(reason) => {
                    if let Some(line) = self.line(at, &reason)? {
                        progress::step(line);
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
        let standing = standing_of(self.launch, &self.candidates[at])?;
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
                let minutes = SETTLE.num_minutes();
                format!("not settled: {shaped} less than {minutes} minutes ago")
            }
        };
        Ok(Some(format!("#{number} {why}")))
    }
}

/// Where `candidate`, an open issue labelled `ready-for-agent` in the
/// repository of the Launch directory `launch`, stands. It is a Ready issue
/// when it has no label that makes an Unready Ticket and no Claim, is not a
/// sub-issue, has no `base-fix` label and no open blocker, was never started,
/// is not a Spec whose Tickets are all closed, and is settled: [`SETTLE`] has
/// passed since it was last labelled `ready-for-agent` and since a sub-issue
/// or a "blocked by" link of its was last added or removed.
fn standing_of(launch: &Git, candidate: &ListedIssue) -> Result<Standing> {
    let passed_over = |reason| Ok(Standing::PassedOver(reason));
    if let Some(label) = candidate.labels.unready() {
        return passed_over(Reason::Unready(label));
    }
    if claim::is_on(&candidate.labels) {
        return passed_over(Reason::Claimed);
    }
    let read = github::candidate(&candidate.issue, READY_FOR_AGENT)?;
    if let Some(spec) = read.parent {
        return passed_over(Reason::Ticket(spec));
    }
    if base_fix::is_issue(&candidate.labels) {
        return passed_over(Reason::BaseFix);
    }
    if !read.open_blockers.is_empty() {
        return passed_over(Reason::Blocked(read.open_blockers));
    }
    if let Some(started) = branch::started(launch, &candidate.issue)? {
        return passed_over(Reason::Started(started));
    }
    if spec_run::all_closed(read.sub_issue_is_open.iter().copied()) {
        return passed_over(Reason::TicketsClosed);
    }
    if let Some(shaped) = read.last_shaped
        && Utc::now() - shaped.at < SETTLE
    {
        return passed_over(Reason::Unsettled(shaped.by));
    }
    Ok(Standing::Ready {
        is_spec: read.has_sub_issues(),
    })
}
