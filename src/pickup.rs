//! A Pickup run up to the issue it takes: from the Launch directory with no
//! Issue URL, the search for the lowest-numbered Ready issue in the
//! repository, for the command to dispatch, saying why it passed over each
//! issue labelled `ready-for-agent` before it. Only one Pickup run or
//! Architect run per repository runs at a time on a machine: one started
//! while another is still running is skipped, before any search. So is one
//! that finds no Ready issue.

use std::fmt;

use anyhow::Result;
use chrono::{TimeDelta, Utc};

use crate::base_fix;
use crate::branch::{self, Started};
use crate::claim;
use crate::git::Git;
use crate::github::{self, ListedIssue, Shaping};
use crate::issue::{IssueUrl, Repo};
use crate::launch::{self, AlreadyRunning, Launch, Start};
use crate::progress;
use crate::spec_run::{self, READY_FOR_AGENT};

/// How a Pickup run ended, short of a failure and before any dispatch.
pub enum Outcome {
    /// It took this Ready issue, to dispatch.
    Taken(Taken),
    /// Skipped, having done nothing.
    Skipped(Skipped),
}

/// The Ready issue a Pickup run took.
pub struct Taken {
    pub issue: IssueUrl,
    /// The Pickup run's Base branch, which the run the issue is dispatched
    /// as takes.
    pub base: String,
    /// Whether it is a Spec, an issue with sub-issues, which is dispatched
    /// as a Spec run.
    pub is_spec: bool,
}

/// Why a Pickup run was skipped. Its `Display` is the reason, as the skipped
/// run's one line gives it.
pub enum Skipped {
    /// An Architect run or another Pickup run on this repository is still
    /// running on this machine, the Spec run or Run it dispatched included.
    AlreadyRunning(AlreadyRunning),
    /// The repository has no Ready issue.
    NoReadyIssue(Repo),
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::AlreadyRunning(running) => running.fmt(f),
            Self::NoReadyIssue(repo) => write!(f, "no Ready issue on {}", repo.slug()),
        }
    }
}

/// Take the lowest-numbered Ready issue in the Launch directory's
/// repository, saying which on stderr, after a line there on each issue
/// labelled `ready-for-agent` it passed over on the way, with why. The Base
/// branch is `base`, the branch the command named, whatever the Launch
/// directory has checked out, or without one the branch checked out there.
/// Nothing is changed, on GitHub or in the Launch directory: the run the
/// issue is dispatched as makes the Claim.
///
/// Once the preflight checks pass, and before any search, the Pickup run is
/// skipped if an Architect run or another Pickup run on the same repository
/// is still running on this machine. Otherwise this process is that
/// repository's one such run until it exits, through whatever it dispatches.
pub fn run(base: Option<&str>) -> Result<Outcome> {
    let Launch {
        git, repo, base, ..
    } = match launch::start(base)? {
        Start::Clear(launch) => launch,
        Start::AlreadyRunning(running) => {
            return Ok(Outcome::Skipped(Skipped::AlreadyRunning(running)));
        }
    };
    let candidates = github::open_issues_labelled(&repo.slug(), READY_FOR_AGENT)?;
    let Some((ready, is_spec)) = Search::new(&git, candidates).first_ready()? else {
        return Ok(Outcome::Skipped(Skipped::NoReadyIssue(repo)));
    };
    progress::step(format_args!(
        "taking Ready issue #{} \"{}\", as thirdshift {} would",
        ready.issue.number, ready.title, ready.issue.url
    ));
    Ok(Outcome::Taken(Taken {
        issue: ready.issue,
        base,
        is_spec,
    }))
}

/// How long an issue is left after it was last shaped before a Pickup run
/// takes it, so a Spec is not taken while its Tickets are still being
/// attached. Fixed, with no setting.
const SETTLE: TimeDelta = TimeDelta::minutes(10);

/// Where an open issue labelled `ready-for-agent` stands with a Pickup run.
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
    Unready(&'static str),
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

    /// The lowest-numbered Ready issue, if there is one, and whether it is a
    /// Spec, with a line on stderr for each issue passed over before it.
    fn first_ready(mut self) -> Result<Option<(ListedIssue, bool)>> {
        for at in 0..self.candidates.len() {
            match self.standing(at)? {
                Standing::Ready { is_spec } => {
                    return Ok(Some((self.candidates.swap_remove(at), is_spec)));
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
        let standing = stand(self.launch, &self.candidates[at])?;
        self.standings[at] = Some(standing.clone());
        Ok(standing)
    }

    /// The line on the candidate at `at`, passed over for `reason`. A Ticket
    /// whose Spec is a Ready issue gets none: it is reached through its Spec,
    /// which this Pickup run or a later one takes.
    fn line(&mut self, at: usize, reason: &Reason) -> Result<Option<String>> {
        let number = self.candidates[at].issue.number;
        let why = match reason {
            Reason::Unready(label) => format!("labelled {label}"),
            Reason::Claimed => format!("labelled {}", claim::IN_PROGRESS),
            Reason::Ticket(spec) => {
                let listed = self.candidates.iter().position(|candidate| {
                    candidate.issue.number == spec.number && same_repo(&candidate.issue, spec)
                });
                let spec_is_ready = match listed {
                    Some(spec) => matches!(self.standing(spec)?, Standing::Ready { .. }),
                    None => false,
                };
                if spec_is_ready {
                    return Ok(None);
                }
                let spec = if same_repo(&self.candidates[at].issue, spec) {
                    format!("#{}", spec.number)
                } else {
                    format!("{}#{}", spec.repo_slug(), spec.number)
                };
                format!("is a Ticket of {spec}, which is not ready")
            }
            Reason::BaseFix => format!("labelled {}", base_fix::BASE_FIX_LABEL),
            Reason::Blocked(blockers) => {
                let blockers: Vec<String> = blockers
                    .iter()
                    .map(|blocker| format!("#{blocker}"))
                    .collect();
                format!("blocked by {}", blockers.join(", "))
            }
            Reason::Started(started) => format!("already started: {started}"),
            Reason::Unsettled(shaping) => {
                let shaped = match shaping {
                    Shaping::Labelled => format!("labelled {READY_FOR_AGENT}"),
                    Shaping::SubIssues => "a sub-issue added or removed".to_string(),
                    Shaping::Blockers => "a \"blocked by\" link added or removed".to_string(),
                };
                format!("not settled: {shaped} less than ten minutes ago")
            }
        };
        Ok(Some(format!("#{number} {why}")))
    }
}

/// Whether `one` and `other` are in the same repository, whatever the case
/// their URLs spell it in.
fn same_repo(one: &IssueUrl, other: &IssueUrl) -> bool {
    one.owner.eq_ignore_ascii_case(&other.owner) && one.repo.eq_ignore_ascii_case(&other.repo)
}

/// Where `candidate`, an open issue labelled `ready-for-agent` in the
/// repository of the Launch directory `launch`, stands. It is a Ready issue
/// when it has no label that makes an Unready Ticket, no Claim, no parent
/// issue, no `base-fix` label and no open blocker, was never started, and is
/// settled: [`SETTLE`] has passed since it was last labelled
/// `ready-for-agent` and since a sub-issue or a "blocked by" link of its was
/// last added or removed.
fn stand(launch: &Git, candidate: &ListedIssue) -> Result<Standing> {
    let passed_over = |reason| Ok(Standing::PassedOver(reason));
    if let Some(label) = spec_run::unready_label(&candidate.labels) {
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
    if let Some(shaped) = read.last_shaped
        && Utc::now() - shaped.at < SETTLE
    {
        return passed_over(Reason::Unsettled(shaped.by));
    }
    Ok(Standing::Ready {
        is_spec: read.has_sub_issues,
    })
}
