//! The Failed run: why a Run can't end with a ready PR. The Delivery takes
//! a Run down the Failed run path; this is what that path ends with.

use std::fmt;
use std::path::PathBuf;

use anyhow::anyhow;

use crate::interrupt;

/// Why a Run did not end with a ready PR, and what the user should see.
pub struct FailedRun {
    pub error: anyhow::Error,
    /// The Issue branch's open PR, if it has one.
    pub pr_url: Option<String>,
    /// The most recent session log, if a session was started.
    pub log: Option<PathBuf>,
    /// Whether the Run was interrupted, as it was when the Run failed.
    pub interrupted: bool,
    /// In a Spec run, a line on each Ticket it landed or did not get done;
    /// empty in a Run.
    pub ticket_lines: Vec<String>,
}

/// A merge GitHub refused when a round of the Repair loop found nothing to
/// fix. The Run fails, but, as it can do no more, leaves the PR ready for review
/// rather than a draft. It is the context of the merge error.
#[derive(Debug)]
pub struct PolicyRefusal;

impl fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "the merge was refused with nothing left to fix, so the PR stays ready for review",
        )
    }
}

/// A failure with nothing to push or clean up, as before the worktree exists.
impl From<anyhow::Error> for FailedRun {
    fn from(error: anyhow::Error) -> Self {
        let interrupted = interrupt::requested();
        FailedRun {
            error: interrupted_or(error, interrupted),
            pr_url: None,
            log: None,
            interrupted,
            ticket_lines: Vec::new(),
        }
    }
}

/// `interrupted` if the Run was, else `error`: an interrupt can surface as
/// some other error, such as a killed git.
pub fn interrupted_or(error: anyhow::Error, interrupted: bool) -> anyhow::Error {
    if interrupted {
        anyhow!("interrupted")
    } else {
        error
    }
}
