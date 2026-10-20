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
const RETAINED_WORK: &str = "Retained work: ";
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
    BaseFix,
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
            Self::BaseFix => "Base fix",
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

/// Closed/total Tickets from the last lifecycle graph, as the Ticket board
/// reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TicketProgress {
    pub landed: usize,
    pub total: usize,
}

/// The facts each layer of a Failed run adds, from the Delivery up to the
/// Run's end; the rules for how each fact changes the lines live here.
#[derive(Debug, PartialEq, Eq)]
pub struct Recovery {
    stage: Stage,
    preservation: Preservation,
    worktree: Option<PathBuf>,
    tickets: Option<TicketProgress>,
    /// Whether the Spec review completed, independently of any later
    /// delivery failure.
    reviewed: bool,
    continuation: Option<Continuation>,
    /// Factory-reported retention facts from child Runs, including their paths.
    local_child_work: Vec<String>,
}

impl Recovery {
    pub fn new(stage: Stage, preservation: Preservation) -> Self {
        Self {
            stage,
            preservation,
            worktree: None,
            tickets: None,
            reviewed: false,
            continuation: None,
            local_child_work: Vec::new(),
        }
    }

    /// Only tests read the stage: its rules live here.
    #[cfg(test)]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Record `path` as the worktree that holds work preserved only locally.
    pub fn record_worktree(&mut self, path: PathBuf) {
        self.worktree = Some(path);
    }

    /// Add Ticket `progress`; the final review is complete only past the
    /// Ticket and Spec review stages, and only if `all_accounted`: every
    /// Ticket was accounted for.
    pub fn add_tickets(&mut self, progress: TicketProgress, all_accounted: bool) {
        self.tickets = Some(progress);
        self.reviewed = !matches!(self.stage, Stage::Tickets | Stage::SpecReview) && all_accounted;
    }

    /// Add the public command `context`. A Continuation whose Issue branch
    /// was already on origin, `on_origin`, keeps that work there even when
    /// nothing more was preserved.
    pub fn add_continuation(&mut self, context: Continuation, on_origin: bool) {
        if self.preservation == Preservation::None && on_origin {
            self.preservation = Preservation::Remote;
        }
        self.continuation = Some(context);
    }

    /// Add child Runs' retained `work`, after any already added.
    pub fn add_child_work(&mut self, work: impl IntoIterator<Item = String>) {
        self.local_child_work.extend(work);
    }

    /// Literal lines shared by terminal, Activity and Run notification output.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("{STOPPED}{}", self.stage.name())];
        if let Some(TicketProgress { landed, total }) = self.tickets {
            let review = if self.reviewed {
                "complete"
            } else {
                "incomplete"
            };
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
            self.local_child_work
                .iter()
                .map(|line| format!("{RETAINED_WORK}{line}")),
        );
        if self.preservation == Preservation::Local || !self.local_child_work.is_empty() {
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

    /// Read only factory-owned retention facts, including those passed through
    /// a Ticket from its Base fix. Agent output never enters this protocol.
    pub fn local_work(line: &str) -> Option<&str> {
        let line = line.strip_prefix(RETAINED_WORK).unwrap_or(line);
        line.strip_prefix(PRESERVED)
            .is_some_and(|value| value.starts_with(LOCAL_WORK))
            .then_some(line)
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
            RETAINED_WORK,
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Continuation {
        Continuation {
            command: "thirdshift https://github.com/acme/widgets/issues/21".into(),
            launch: "/launch".into(),
            base: "main".into(),
            branch: "issue-21".into(),
        }
    }

    fn preserved(recovery: &Recovery) -> String {
        recovery
            .lines()
            .into_iter()
            .find(|line| line.starts_with(PRESERVED))
            .expect("no Preserved line")
    }

    fn has(recovery: &Recovery, prefix: &str) -> bool {
        recovery.lines().iter().any(|line| line.starts_with(prefix))
    }

    #[test]
    fn a_continuation_with_nothing_pushed_reads_preserved_remotely() {
        let mut recovery = Recovery::new(Stage::Implementation, Preservation::None);
        recovery.add_continuation(context(), true);
        assert_eq!(preserved(&recovery), "Preserved: remotely on issue-21");
        assert!(has(&recovery, CONTINUING));
        assert!(!has(&recovery, RETRY));
    }

    #[test]
    fn a_fresh_run_with_nothing_pushed_reads_no_work_preserved() {
        let mut recovery = Recovery::new(Stage::Implementation, Preservation::None);
        recovery.add_continuation(context(), false);
        assert_eq!(preserved(&recovery), "Preserved: no work preserved");
        assert!(has(&recovery, RETRY));
        assert!(!has(&recovery, CONTINUING));
    }

    #[test]
    fn the_final_review_is_complete_only_past_the_spec_review_with_every_ticket_accounted() {
        let progress = TicketProgress {
            landed: 3,
            total: 3,
        };
        for (stage, all_accounted, review) in [
            (Stage::Tickets, true, "incomplete"),
            (Stage::SpecReview, true, "incomplete"),
            (Stage::Push, false, "incomplete"),
            (Stage::Push, true, "complete"),
            (Stage::Ci, true, "complete"),
        ] {
            let mut recovery = Recovery::new(stage, Preservation::Remote);
            recovery.add_tickets(progress, all_accounted);
            assert_eq!(
                recovery.lines()[1],
                format!("Progress: 3/3 Tickets landed; final review {review}"),
                "{stage:?}, all accounted: {all_accounted}"
            );
        }
    }

    #[test]
    fn retained_work_from_two_layers_is_kept_in_order() {
        let mut recovery = Recovery::new(Stage::Ci, Preservation::Remote);
        recovery.add_continuation(context(), false);
        recovery.add_child_work(["Ticket #22 kept at /kept/22".to_string()]);
        recovery.add_child_work(["Base fix #30 kept at /kept/30".to_string()]);
        let lines = recovery.lines();
        let retained: Vec<_> = lines
            .iter()
            .filter(|line| line.starts_with(RETAINED_WORK))
            .collect();
        assert_eq!(
            retained,
            [
                "Retained work: Ticket #22 kept at /kept/22",
                "Retained work: Base fix #30 kept at /kept/30"
            ]
        );
        assert!(has(&recovery, BEFORE_RETRYING));
    }

    #[test]
    fn local_preservation_names_the_recorded_worktree_or_else_the_launch_directory() {
        let mut recovery = Recovery::new(Stage::Push, Preservation::Local);
        recovery.add_continuation(context(), false);
        assert_eq!(
            preserved(&recovery),
            "Preserved: work may exist only locally on issue-21 at /launch; push was not confirmed"
        );
        recovery.record_worktree("/worktrees/issue-21".into());
        assert_eq!(
            preserved(&recovery),
            "Preserved: work may exist only locally on issue-21 at /worktrees/issue-21; push was not confirmed"
        );
        assert!(has(&recovery, BEFORE_RETRYING));
    }
}
