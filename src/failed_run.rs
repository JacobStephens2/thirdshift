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

/// A Self-merge blocked by policy. The Run fails but leaves the PR ready
/// for review, with no failure commit pushed onto the watched head.
#[derive(Debug)]
pub enum PolicyRefusal {
    /// A refused merge whose Repair round found nothing to fix.
    MergeRefused,
    /// A structured review decision blocks the merge before any attempt.
    ReviewRequired { url: String },
}

impl std::error::Error for PolicyRefusal {}

impl fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MergeRefused => f.write_str(
                "the merge was refused with nothing left to fix, so the PR stays ready for review",
            ),
            Self::ReviewRequired { url } => write!(
                f,
                "PR {url} needs an approving review before it can merge; it is ready for review"
            ),
        }
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
