//! Deterministic facts about unfinished work, shared by every ending renderer.

use std::path::PathBuf;

use crate::harness::Harness;

// Shared by rendering and child Run accounting: one owner for the line protocol.
const STOPPED: &str = "Stopped at: ";
const PROGRESS: &str = "Progress: ";
const PRESERVED: &str = "Preserved: ";
const BEFORE_RETRYING: &str = "Before retrying: ";
const RECOVERY: &str = "Recovery: ";
const SECURITY_REVIEW: &str = "Security review will use ";
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

/// Where a preserve attempt left the work. A push that fails is local, and
/// `report` receives the failed-ticket-run line.
pub fn preservation_of(
    result: Result<crate::worktree::Preserved, anyhow::Error>,
    report: impl FnOnce(String),
) -> Preservation {
    match result {
        Ok(crate::worktree::Preserved::Pushed) => Preservation::Remote,
        Ok(crate::worktree::Preserved::NoChanges) => Preservation::None,
        Err(problem) => {
            report(format!(
                "could not push the failed ticket run's work, so it may exist only locally: {problem:#}"
            ));
            Preservation::Local
        }
    }
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
    /// Ticket progress, and whether the Spec review completed,
    /// independently of any later delivery failure.
    tickets: Option<(TicketProgress, bool)>,
    continuation: Option<Continuation>,
    security_review_harness: Option<Harness>,
    /// Factory-reported retention facts from child Runs, including their paths.
    local_child_work: Vec<String>,
    /// The Spec PR changed under the Spec run, so no command resumes it.
    spec_pr_changed: bool,
    fix_run: bool,
}

impl Recovery {
    pub fn new(stage: Stage, preservation: Preservation) -> Self {
        Self {
            stage,
            preservation,
            worktree: None,
            tickets: None,
            continuation: None,
            security_review_harness: None,
            local_child_work: Vec::new(),
            spec_pr_changed: false,
            fix_run: false,
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
        let reviewed = !matches!(self.stage, Stage::Tickets | Stage::SpecReview) && all_accounted;
        self.tickets = Some((progress, reviewed));
    }

    /// Add the public command `context` that continues or retries the Run.
    /// A **Continuation**, whose Issue branch was already on origin,
    /// `on_origin`, keeps that work there even when nothing more was preserved.
    pub fn add_command_context(&mut self, context: Continuation, on_origin: bool) {
        if self.preservation == Preservation::None && on_origin {
            self.preservation = Preservation::Remote;
        }
        self.continuation = Some(context);
    }

    pub(crate) fn for_fix_run(&mut self) {
        self.fix_run = true;
    }

    /// Add child Runs' retained `work`, after any already added.
    pub fn add_child_work(&mut self, work: impl IntoIterator<Item = String>) {
        self.local_child_work.extend(work);
    }

    /// The Spec PR was closed, merged or retargeted under the Spec run: no
    /// Recovery command resumes its Spec branch.
    pub fn add_spec_pr_changed(&mut self) {
        self.spec_pr_changed = true;
    }

    /// The Security review's Harness when `security.harness` chose one
    /// different from the Run's; the Recovery command still names the Run's.
    pub fn add_security_review_harness(&mut self, harness: Harness) {
        self.security_review_harness = Some(harness);
    }

    /// Literal lines shared by terminal, Activity and Run notification output.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("{STOPPED}{}", self.stage.name())];
        if let Some((TicketProgress { landed, total }, reviewed)) = self.tickets {
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
            self.local_child_work
                .iter()
                .map(|line| format!("{RETAINED_WORK}{line}")),
        );
        if self.preservation == Preservation::Local || !self.local_child_work.is_empty() {
            lines.push(format!("{BEFORE_RETRYING}inspect and publish the retained work, reconcile with origin without forcing, then free the Issue branch and Worktree acquisition path. Do not discard local work."));
        }
        if self.spec_pr_changed {
            lines.push(format!("{BEFORE_RETRYING}the Spec PR changed under the Spec run, so no Recovery command resumes {} into {}. Decide the Spec's Base branch and Spec branch before starting it again.", context.branch, context.base));
            return lines;
        }
        lines.push(format!("{RECOVERY}{}", context.command));
        if self.fix_run {
            lines.push(format!("{LAUNCH}{} or another clone passing Origin match. The Fix run uses the pull request's Base branch {} and continues its Repair loop.", context.launch.display(), context.base));
            return lines;
        }
        if let Some(harness) = self.security_review_harness {
            lines.push(format!(
                "{SECURITY_REVIEW}{}, chosen by security.harness in the User config.",
                harness.name()
            ));
        }
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

    /// The Recovery comment for the issue a Run left `agent-failed`, for the
    /// Day shift reading GitHub: the cause, where it stopped, what was
    /// preserved and the machine, without local paths or the continue
    /// command. See [`comment`].
    pub fn comment(&self, cause: &str, pr_url: Option<&str>, machine: Option<&str>) -> String {
        let mut facts = vec![format!("{STOPPED}{}", self.stage.name())];
        if let Some((TicketProgress { landed, total }, reviewed)) = self.tickets {
            let review = if reviewed { "complete" } else { "incomplete" };
            facts.push(format!(
                "{PROGRESS}{landed}/{total} Tickets landed; final review {review}"
            ));
        }
        let branch = self.continuation.as_ref().map(|context| &context.branch);
        let on = machine.unwrap_or("the machine that ran it");
        let mut preserved = match (self.preservation, branch) {
            (Preservation::Remote, Some(branch)) => format!("Issue branch `{branch}` on origin"),
            (Preservation::Local, Some(branch)) => format!(
                "Issue branch `{branch}` may exist only locally on {on}; its push was not confirmed"
            ),
            (Preservation::None, Some(_)) | (_, None) => "no work".to_string(),
        };
        if let Some(pr_url) = pr_url {
            preserved += &format!(", with pull request {pr_url}");
        }
        if !self.local_child_work.is_empty() {
            preserved += &format!("; some child work may exist only locally on {on}");
        }
        facts.push(format!("{PRESERVED}{preserved}"));
        render(cause, facts, machine)
    }

    pub fn is_line(line: &str) -> bool {
        [
            STOPPED,
            PROGRESS,
            PRESERVED,
            BEFORE_RETRYING,
            RECOVERY,
            SECURITY_REVIEW,
            LAUNCH,
            CONTINUING,
            RETRY,
            RETAINED_WORK,
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }
}

/// The AI-attribution line every comment thirdshift writes opens with.
pub const GENERATED: &str = "This comment was generated by AI.";

/// The Recovery comment for a Run that failed with no [`Recovery`]: it
/// stopped before its work began, so it names only the Issue branch
/// `branch` and its pull request `pr_url` as preserved, if there are any.
pub fn comment(
    recovery: Option<&Recovery>,
    branch: Option<&str>,
    cause: &str,
    pr_url: Option<&str>,
    machine: Option<&str>,
) -> String {
    if let Some(recovery) = recovery {
        return recovery.comment(cause, pr_url, machine);
    }
    let preserved = match (branch, pr_url) {
        (Some(branch), Some(pr_url)) => {
            format!("Issue branch `{branch}`, with pull request {pr_url}")
        }
        (Some(branch), None) => format!("Issue branch `{branch}`, if it reached origin"),
        (None, Some(pr_url)) => format!("pull request {pr_url}"),
        (None, None) => "no work".to_string(),
    };
    render(cause, vec![format!("{PRESERVED}{preserved}")], machine)
}

/// The comment's text: the attribution line, `cause` with local paths taken
/// out, `facts` and the `machine`, then how to retry.
fn render(cause: &str, facts: Vec<String>, machine: Option<&str>) -> String {
    let cause = without_local_paths(&cause.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut text = format!(
        "{GENERATED}\n\nthirdshift's Run on this issue failed, so it waits for the Day shift.\n\n\
         - Cause: {cause}\n"
    );
    for fact in facts {
        text += &format!("- {fact}\n");
    }
    text += &format!(
        "- Machine: {}\n\nPut `ready-for-agent` back to retry.\n",
        machine.unwrap_or("unknown")
    );
    text
}

/// `text` with each word that is an absolute or home-relative local path,
/// past any opening quote or bracket, replaced by `<local path>`, keeping
/// the punctuation around it. A URL is not a local path.
fn without_local_paths(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let start = word.len() - word.trim_start_matches(['(', '[', '\'', '"', '`']).len();
            let rest = &word[start..];
            if !(rest.starts_with('/') || rest.starts_with("~/")) {
                return word.to_string();
            }
            let path = rest.trim_end_matches([')', ']', '\'', '"', '`', ',', '.', ';', ':']);
            format!("{}<local path>{}", &word[..start], &rest[path.len()..])
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        recovery.add_command_context(context(), true);
        assert_eq!(preserved(&recovery), "Preserved: remotely on issue-21");
        assert!(has(&recovery, CONTINUING));
        assert!(!has(&recovery, RETRY));
    }

    #[test]
    fn a_spec_pr_changed_under_the_spec_run_leaves_out_every_command_line() {
        let mut recovery = Recovery::new(Stage::Tickets, Preservation::Remote);
        recovery.add_command_context(context(), true);
        recovery.add_spec_pr_changed();
        assert_eq!(preserved(&recovery), "Preserved: remotely on issue-21");
        assert!(has(&recovery, BEFORE_RETRYING));
        for command in [RECOVERY, LAUNCH, CONTINUING, RETRY] {
            assert!(!has(&recovery, command), "{command}");
        }
    }

    #[test]
    fn a_fresh_run_with_nothing_pushed_reads_no_work_preserved() {
        let mut recovery = Recovery::new(Stage::Implementation, Preservation::None);
        recovery.add_command_context(context(), false);
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
        recovery.add_command_context(context(), false);
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
        recovery.add_command_context(context(), false);
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

    #[test]
    fn the_comment_names_the_cause_stop_preserved_branch_and_pull_request_machine_and_retry() {
        let mut recovery = Recovery::new(Stage::Ci, Preservation::Remote);
        recovery.add_command_context(context(), false);
        assert_eq!(
            recovery.comment(
                "CI failed",
                Some("https://github.com/acme/widgets/pull/3"),
                Some("night-host")
            ),
            "This comment was generated by AI.\n\n\
             thirdshift's Run on this issue failed, so it waits for the Day shift.\n\n\
             - Cause: CI failed\n\
             - Stopped at: CI watching\n\
             - Preserved: Issue branch `issue-21` on origin, with pull request \
             https://github.com/acme/widgets/pull/3\n\
             - Machine: night-host\n\n\
             Put `ready-for-agent` back to retry.\n"
        );
    }

    #[test]
    fn the_comment_leaves_out_local_paths_and_the_continue_command() {
        let mut recovery = Recovery::new(Stage::Push, Preservation::Local);
        recovery.add_command_context(context(), false);
        recovery.record_worktree("/worktrees/issue-21".into());
        recovery.add_child_work(["Ticket #22 kept at /kept/22".to_string()]);
        let comment = recovery.comment(
            "could not push (/worktrees/issue-21): see ~/.thirdshift/logs/a.jsonl,\nthen retry",
            None,
            Some("night-host"),
        );
        for local in [
            "/worktrees",
            "/kept",
            "~/",
            "/launch",
            "thirdshift https://",
        ] {
            assert!(!comment.contains(local), "{local}: {comment}");
        }
        assert!(
            comment
                .contains("- Cause: could not push (<local path>): see <local path>, then retry\n"),
            "{comment}"
        );
        assert!(
            comment.contains(
                "- Preserved: Issue branch `issue-21` may exist only locally on night-host; its \
                 push was not confirmed; some child work may exist only locally on night-host\n"
            ),
            "{comment}"
        );
    }

    #[test]
    fn a_comment_without_a_recovery_names_the_branch_and_pull_request_it_has() {
        let comment = comment(
            None,
            Some("issue-21"),
            "could not create the worktree",
            Some("https://github.com/acme/widgets/pull/3"),
            None,
        );
        assert!(
            comment.contains(
                "- Preserved: Issue branch `issue-21`, with pull request \
                 https://github.com/acme/widgets/pull/3\n- Machine: unknown\n"
            ),
            "{comment}"
        );
        assert!(!comment.contains("Stopped at"), "{comment}");
    }
}
