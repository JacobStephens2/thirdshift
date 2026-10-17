//! Deterministic lifecycle facts and shared recovery guidance for a Failed run.

use std::path::PathBuf;

/// The operation that had not finished when the Run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Implementation,
    Tickets,
    SpecReview,
    SecurityReview,
    Synchronization,
    Readiness,
    Repair,
    Ci,
    SelfMerge,
    BaseFix,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Implementation => "Ticket execution",
            Self::Tickets => "Ticket execution",
            Self::SpecReview => "Spec review",
            Self::SecurityReview => "Security review",
            Self::Synchronization => "Branch synchronization",
            Self::Readiness => "PR readiness",
            Self::Repair => "Repair",
            Self::Ci => "CI watching",
            Self::SelfMerge => "Self-merge",
            Self::BaseFix => "Base fix",
        }
    }
}

/// What preservation actually achieved, rather than what an agent claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preservation {
    Remote,
    Local,
    None,
}

/// Counts from the Ticket board, including Tickets completed before this Continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketProgress {
    pub landed: usize,
    pub total: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Recovery {
    pub stage: Stage,
    pub preservation: Preservation,
    pub tickets: Option<TicketProgress>,
    pub local_worktree: Option<PathBuf>,
    /// Executable public command and its Launch directory/Base branch requirement.
    pub continuation: Option<String>,
    pub base: Option<String>,
}

impl Recovery {
    pub fn new(stage: Stage, preservation: Preservation) -> Self {
        Self {
            stage,
            preservation,
            tickets: None,
            local_worktree: None,
            continuation: None,
            base: None,
        }
    }

    /// The same labelled rows go to the terminal and Run notification.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![("Stopped at", self.stage.name().to_string())];
        if let Some(tickets) = self.tickets {
            let review = if self.stage == Stage::SpecReview {
                "; final review incomplete"
            } else {
                ""
            };
            rows.push((
                "Progress",
                format!(
                    "{}/{} Tickets landed{review}",
                    tickets.landed, tickets.total
                ),
            ));
        }
        let preserved = match self.preservation {
            Preservation::Remote => {
                "Work preserved remotely on the Issue branch; reuse its open PR if present."
            }
            Preservation::Local => {
                "Preservation did not finish; work may exist only locally. Recover and push the retained Issue branch before continuing."
            }
            Preservation::None => "No work preserved by this Run.",
        };
        rows.push(("Preservation", preserved.into()));
        if let Some(path) = &self.local_worktree {
            rows.push(("Local worktree", path.display().to_string()));
        }
        if let Some(command) = &self.continuation {
            let advice = match self.preservation {
                Preservation::Remote => {
                    "Re-run when the cause is resolved or usage is available. Continuation reuses the preserved branch and open PR, and skips completed Tickets. It starts a new agent session; further merge or CI Repairs may still be needed."
                }
                Preservation::Local => {
                    "After recovering and pushing local work, re-run when the cause is resolved. Continuation reuses the branch and open PR and skips completed Tickets; it starts a new agent session and may need further merge or CI Repairs."
                }
                Preservation::None => {
                    "Re-run when the cause is resolved; there is no preserved work from this Run to continue."
                }
            };
            rows.push(("Recovery", advice.into()));
            if let Some(base) = &self.base {
                rows.push(("Launch", format!("Run from a Launch directory whose origin matches the Issue URL, with Base branch {base} checked out. An open Continuation PR supplies its Base branch.")));
            }
            rows.push(("Continue", command.clone()));
        }
        rows
    }

    /// Child ending readers must not mistake recovery rows for a failure cause.
    pub fn is_line(line: &str) -> bool {
        [
            "Stopped at:",
            "Progress:",
            "Preservation:",
            "Local worktree:",
            "Recovery:",
            "Launch:",
            "Continue:",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }
}
