//! The Failed run: why a Run can't end with a ready PR. The Delivery takes
//! a Run down the Failed run path; this is what that path ends with.

use std::fmt;
use std::path::PathBuf;

use anyhow::anyhow;

use crate::interrupt;

/// Why a Run did not end with a ready PR, and what the user should see.
pub struct FailedRun {
    pub recovery: Option<Box<crate::recovery::Recovery>>,
    pub error: anyhow::Error,
    /// The Issue branch's open PR, if it has one.
    pub pr_url: Option<String>,
    /// The Session trace of the session the failure came from, else the most
    /// recent one, if a session was started.
    pub log: Option<PathBuf>,
    /// Whether the Run was interrupted, as it was when the Run failed.
    pub interrupted: bool,
    /// In a Spec run, a line on each Ticket it landed or did not get done;
    /// empty in a Run.
    pub ticket_lines: Vec<String>,
}

impl FailedRun {
    /// Whether a policy, security or merge hold held its Self-merge, leaving
    /// its pull request ready for review on the head whose CI was watched.
    pub fn held(&self) -> bool {
        is_hold(&self.error)
    }

    /// Whether it is a Spec run that ended only waiting, uninterrupted: every
    /// Ticket not done waits on a person or on other work, and none failed.
    pub fn waiting(&self) -> bool {
        self.waiting_on().is_some()
    }

    /// The Tickets not done a Spec run that ended only waiting waits on.
    fn waiting_on(&self) -> Option<&crate::spec_run::TicketsNotDone> {
        self.error
            .downcast_ref::<crate::spec_run::TicketsNotDone>()
            .filter(|not_done| !self.interrupted && not_done.only_waiting)
    }

    /// The comment its Claim's ending posts on its issue, from the Issue
    /// branch `branch` if known: a waiting Spec run's names each Ticket not
    /// done and what it waits on, and any other's is its Recovery comment.
    pub fn comment(&self, branch: Option<&str>) -> String {
        if let Some(not_done) = self.waiting_on() {
            return not_done.comment();
        }
        crate::recovery::comment(
            self.recovery.as_deref(),
            branch,
            &crate::recovery::cause(&self.error),
            self.pr_url.as_deref(),
            crate::host::name().as_deref(),
        )
    }
}

/// The failure category a Command record keeps for an ending that failed
/// with `error`. A hold or a waiting Spec run is the error itself; a failed
/// Harness Session may be wrapped by the stage that ran it.
pub fn category(error: &anyhow::Error) -> crate::logs::FailureCategory {
    use crate::logs::FailureCategory;
    if is_hold(error) {
        FailureCategory::Held
    } else if error
        .downcast_ref::<crate::spec_run::TicketsNotDone>()
        .is_some_and(|not_done| not_done.only_waiting)
    {
        FailureCategory::Waiting
    } else if error
        .chain()
        .any(|layer| layer.is::<crate::harness::interpretation::SessionFailure>())
    {
        FailureCategory::Crashed
    } else {
        FailureCategory::StageFailed
    }
}

/// The Command log line for a failure [`out_of_disk`] recognises.
pub const OUT_OF_DISK: &str =
    "the host ran out of disk space; free disk space on it, then retry the Command";

/// Whether the host running out of disk space caused `error`: a layer of its
/// cause chain is the OS's error for no space left, or says so as the OS and
/// git word it, as a failed `git` command's output does. Such a failure is
/// the host's, not its issue's.
pub fn out_of_disk(error: &anyhow::Error) -> bool {
    error.chain().any(|layer| {
        layer
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull)
            || says_out_of_disk(&layer.to_string())
    })
}

/// Whether the rendered cause `text` says the host ran out of disk space, as
/// the OS and git word it.
pub fn says_out_of_disk(text: &str) -> bool {
    text.contains("No space left on device")
}

/// Whether `error` is a policy, security or merge hold: a Self-merge held
/// with its pull request already ready.
pub fn is_hold(error: &anyhow::Error) -> bool {
    error.is::<PolicyRefusal>()
        || error.is::<crate::security::review::Hold>()
        || error.is::<MergeHold>()
}

/// A repository policy that prevents Self-merge of a ready PR. The Run
/// fails without pushing a failure commit or converting the PR to a draft.
#[derive(Debug)]
pub enum PolicyRefusal {
    /// A refused merge whose next Repair round found nothing to fix.
    NothingToFix,
    /// A structured review decision that blocks the merge before a request.
    ReviewRequired { pr_url: String },
}

impl std::error::Error for PolicyRefusal {}

impl fmt::Display for PolicyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NothingToFix => f.write_str(
                "the merge was refused with nothing left to fix, so the PR stays ready for review",
            ),
            Self::ReviewRequired { pr_url } => write!(
                f,
                "PR {pr_url} needs an approving review before it can merge; it is ready for review"
            ),
        }
    }
}

/// A Self-merge held before the merge request, with the pull request already
/// ready. The run fails without drafting the pull request. A protected path
/// in a Custom process is one; `reason` names the path.
#[derive(Debug)]
pub struct MergeHold {
    pub reason: String,
}

impl std::error::Error for MergeHold {}

impl fmt::Display for MergeHold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

/// A failure with nothing to push or clean up, as before the worktree exists.
impl From<anyhow::Error> for FailedRun {
    fn from(error: anyhow::Error) -> Self {
        let interrupted = interrupt::requested();
        FailedRun {
            recovery: None,
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

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;
    use crate::logs::FailureCategory;

    #[test]
    fn the_failure_category_comes_from_the_typed_error_not_its_words() {
        let crashed = Err::<(), _>(anyhow::Error::new(
            crate::harness::interpretation::SessionFailure::exited("claude", 1),
        ))
        .context("Code review failed")
        .unwrap_err();
        for (error, category) in [
            (
                anyhow::Error::new(MergeHold {
                    reason: "protected path".into(),
                }),
                FailureCategory::Held,
            ),
            (
                anyhow::Error::new(PolicyRefusal::NothingToFix),
                FailureCategory::Held,
            ),
            (
                anyhow::Error::new(crate::spec_run::TicketsNotDone::waiting()),
                FailureCategory::Waiting,
            ),
            (crashed, FailureCategory::Crashed),
            // A cause that only names a hold or a crash is not one.
            (
                anyhow!("held: claude exited 1"),
                FailureCategory::StageFailed,
            ),
        ] {
            assert_eq!(super::category(&error), category, "{error:#}");
        }
    }

    #[test]
    fn running_out_of_disk_is_recognised_anywhere_in_the_cause_chain() {
        let full = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::ENOSPC))
            .context("could not write Session trace");
        let git = anyhow!(
            "git write-tree failed: error: unable to write loose object file: No space left on \
             device"
        )
        .context("could not push the failed ticket run's work");
        assert!(out_of_disk(&full), "{full:#}");
        assert!(out_of_disk(&git), "{git:#}");
        let other = anyhow::Error::new(std::io::Error::from_raw_os_error(libc::EACCES))
            .context("could not write Session trace");
        assert!(!out_of_disk(&other), "{other:#}");
        assert!(!out_of_disk(&anyhow!("claude exited 1")));
    }
}
