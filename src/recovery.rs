//! Deterministic facts about unfinished work, shared by every ending renderer.

use std::path::PathBuf;

// Shared by rendering and child Run accounting: one owner for the line protocol.
const STOPPED: &str = "Stopped at: ";
const PROGRESS: &str = "Progress: ";
const PRESERVED: &str = "Preserved: ";
const BEFORE_RETRYING: &str = "Before retrying: ";
const RECOVERY: &str = "Recovery: ";
const LAUNCH: &str = "Run from the Launch directory ";
const CONTINUING: &str = "Continuing reuses ";
const RETRY: &str = "Retry starts work ";
const TICKET_WORK: &str = "Ticket work: ";
const LOCAL_WORK: &str = "work may exist only locally on ";

/// The lifecycle operation underway when a Run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Implementation,
    Tickets,
    SpecReview,
    SecurityReview,
    Push,
    PullRequest,
    Repair,
    Ci,
    SelfMerge,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Implementation => "Implementation",
            Self::Tickets => "Ticket execution",
            Self::SpecReview => "Spec review",
            Self::SecurityReview => "Security review",
            Self::Push => "Push",
            Self::PullRequest => "Pull request readiness",
            Self::Repair => "Repair",
            Self::Ci => "CI watching",
            Self::SelfMerge => "Self-merge",
        }
    }
}

/// The preservation operation's result, never inferred from agent prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preservation {
    Remote,
    Local,
    None,
}

/// Public command context, filled after Base branch and effective asks settle.
#[derive(Debug, PartialEq, Eq)]
pub struct Continuation {
    pub command: String,
    pub launch: PathBuf,
    pub base: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketProgress {
    pub landed: usize,
    pub total: usize,
    pub reviewed: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Recovery {
    pub stage: Stage,
    pub preservation: Preservation,
    pub worktree: Option<PathBuf>,
    /// Closed/total Tickets from the last lifecycle graph, and whether the
    /// Spec review completed, independently of any later delivery failure.
    pub tickets: Option<TicketProgress>,
    pub continuation: Option<Continuation>,
    /// Factory-reported retention facts from child Tickets, including their paths.
    pub local_ticket_work: Vec<String>,
}

impl Recovery {
    pub fn new(stage: Stage, preservation: Preservation) -> Self {
        Self {
            stage,
            preservation,
            worktree: None,
            tickets: None,
            continuation: None,
            local_ticket_work: Vec::new(),
        }
    }

    /// Literal lines shared by terminal, Activity and Run notification output.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("{STOPPED}{}", self.stage.name())];
        if let Some(TicketProgress {
            landed,
            total,
            reviewed,
        }) = self.tickets
        {
            let review = if reviewed { "complete" } else { "incomplete" };
            lines.push(format!(
                "{PROGRESS}{landed}/{total} Tickets landed; final review {review}"
            ));
        }
        let Some(context) = &self.continuation else {
            return lines;
        };
        lines.push(match self.preservation {
            Preservation::Remote => format!("{PRESERVED}remotely on {}", context.branch),
            Preservation::Local => format!(
                "{PRESERVED}{LOCAL_WORK}{} at {}; push was not confirmed",
                context.branch,
                self.worktree
                    .as_deref()
                    .unwrap_or(&context.launch)
                    .display()
            ),
            Preservation::None => format!("{PRESERVED}no work preserved"),
        });
        lines.extend(
            self.local_ticket_work
                .iter()
                .map(|line| format!("{TICKET_WORK}{line}")),
        );
        if self.preservation == Preservation::Local || !self.local_ticket_work.is_empty() {
            lines.push(format!("{BEFORE_RETRYING}inspect and publish the retained work, reconcile with origin without forcing, then free the Issue branch and Worktree acquisition path. Do not discard local work."));
        }
        lines.push(format!("{RECOVERY}{}", context.command));
        lines.push(format!(
            "{LAUNCH}{} with Base branch {} checked out (the command cannot encode the Base branch). Retry when the failure's cause is resolved.",
            context.launch.display(), context.base
        ));
        if self.preservation != Preservation::None {
            lines.push(format!("{CONTINUING}the preserved branch and PR and skips completed Tickets. This starts a new Continuation, not the interrupted agent conversation; review and merge/CI repairs may still be needed."));
        } else {
            lines.push(format!(
                "{RETRY}on the original Issue URL; there is no preserved work to continue."
            ));
        }
        lines
    }

    /// Read only the factory's own preservation line, never agent output.
    pub fn is_local_work(line: &str) -> bool {
        line.strip_prefix(PRESERVED)
            .is_some_and(|value| value.starts_with(LOCAL_WORK))
    }

    pub fn is_line(line: &str) -> bool {
        [
            STOPPED,
            PROGRESS,
            PRESERVED,
            BEFORE_RETRYING,
            RECOVERY,
            LAUNCH,
            CONTINUING,
            RETRY,
            TICKET_WORK,
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }
}
