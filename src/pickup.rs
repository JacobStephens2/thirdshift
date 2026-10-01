//! A Pickup run up to the issue it takes: from the Launch directory with no
//! Issue URL, the search for the lowest-numbered Ready issue in the
//! repository, for the command to dispatch. Only one Pickup run or Architect
//! run per repository runs at a time on a machine: one started while another
//! is still running is skipped, before any search. So is one that finds no
//! Ready issue.

use std::fmt;

use anyhow::Result;

use crate::branch;
use crate::claim;
use crate::git::Git;
use crate::github::{self, ListedIssue};
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
/// repository, saying which on stderr. The Base branch is `base`, the branch
/// the command named, whatever the Launch directory has checked out, or
/// without one the branch checked out there. Nothing is changed, on GitHub
/// or in the Launch directory: the run the issue is dispatched as makes the
/// Claim.
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
    let mut candidates = github::open_issues_labelled(&repo.slug(), READY_FOR_AGENT)?;
    candidates.sort_by_key(|candidate| candidate.issue.number);
    let mut ready = None;
    for candidate in candidates {
        if is_ready(&git, &candidate)? {
            ready = Some(candidate);
            break;
        }
    }
    let Some(ready) = ready else {
        return Ok(Outcome::Skipped(Skipped::NoReadyIssue(repo)));
    };
    progress::step(format_args!(
        "taking Ready issue #{} \"{}\", as thirdshift {} would",
        ready.issue.number, ready.title, ready.issue.url
    ));
    let is_spec = !github::tickets(&ready.issue)?.is_empty();
    Ok(Outcome::Taken(Taken {
        issue: ready.issue,
        base,
        is_spec,
    }))
}

/// Whether `candidate`, an open issue labelled `ready-for-agent` in the
/// repository of the Launch directory `launch`, is a Ready issue: it has no
/// label that makes an Unready Ticket, no Claim, and was never started.
fn is_ready(launch: &Git, candidate: &ListedIssue) -> Result<bool> {
    Ok(spec_run::unready_label(&candidate.labels).is_none()
        && !claim::is_on(&candidate.labels)
        && !branch::started(launch, &candidate.issue)?)
}
