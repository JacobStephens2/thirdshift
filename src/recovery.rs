//! Deterministic facts about unfinished work, shared by every ending renderer.

use std::path::PathBuf;

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
}

impl Recovery {
    pub fn new(stage: Stage, preservation: Preservation) -> Self {
        Self {
            stage,
            preservation,
            worktree: None,
            tickets: None,
            continuation: None,
        }
    }

    /// Literal lines shared by terminal, Activity and Run notification output.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("Stopped at: {}", self.stage.name())];
        if let Some(TicketProgress {
            landed,
            total,
            reviewed,
        }) = self.tickets
        {
            let review = if reviewed { "complete" } else { "incomplete" };
            lines.push(format!(
                "Progress: {landed}/{total} Tickets landed; final review {review}"
            ));
        }
        let Some(context) = &self.continuation else {
            return lines;
        };
        lines.push(match self.preservation {
            Preservation::Remote => format!("Preserved: remotely on {}", context.branch),
            Preservation::Local => format!(
                "Preserved: work may exist only locally on {} at {}; push was not confirmed",
                context.branch,
                self.worktree
                    .as_deref()
                    .unwrap_or(&context.launch)
                    .display()
            ),
            Preservation::None => "Preserved: no work preserved".to_string(),
        });
        if self.preservation == Preservation::Local {
            lines.push("Before retrying: inspect and publish the retained work, reconcile with origin without forcing, then free the Issue branch and Worktree acquisition path. Do not discard local work.".into());
        }
        lines.push(format!("Recovery: {}", context.command));
        lines.push(format!(
            "Run from the Launch directory {} with Base branch {} checked out (the command cannot encode the Base branch). Retry when the failure's cause is resolved.",
            context.launch.display(), context.base
        ));
        if self.preservation != Preservation::None {
            lines.push("Continuing reuses the preserved branch and PR and skips completed Tickets. This starts a new Continuation, not the interrupted agent conversation; review and merge/CI repairs may still be needed.".into());
        } else {
            lines.push("Retry starts work on the original Issue URL; there is no preserved work to continue.".into());
        }
        lines
    }

    pub fn is_line(line: &str) -> bool {
        [
            "Stopped at: ",
            "Progress: ",
            "Preserved: ",
            "Before retrying: ",
            "Recovery: ",
            "Run from the Launch directory ",
            "Continuing reuses ",
            "Retry starts work ",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }
}
