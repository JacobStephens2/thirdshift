//! The prompts agent sessions are started with.

use crate::ci::{self, FailedChecks};
use crate::issue::IssueUrl;
use crate::labels::{NEEDS_TRIAGE, READY_FOR_AGENT};

/// Every prompt ends with this. Sessions run with `claude -p`, which exits
/// once the agent ends its turn, killing any background task still running.
/// Other sessions on the same machine run the same commands, so a wait on a
/// process name can match theirs and outlast the session's own task.
/// thirdshift can't tell a task the agent gave up on from one it was waiting
/// on, so the agent is to stop the first kind itself, with `TaskStop`: auto
/// mode may deny it a `kill`.
const HEADLESS: &str = "You run headless: nobody is watching, and ending your turn ends the session. \
    Run tests and other long commands in the foreground, raising the Bash timeout if needed. \
    If a command is moved to the background, wait for that task by its own task id or output file, \
    never by process names or patterns (`pgrep`, `ps | grep`, and the like): \
    other sessions on this machine run the same commands. \
    Never end your turn while a background task you depend on is still running: ending the turn kills it. \
    Before ending your turn, stop every background task you no longer need with the `TaskStop` tool, by its task id: \
    a task still running when your turn ends is taken as work you were waiting on.\n";

/// The fresh prompt, for a run that starts a new Issue branch.
pub fn fresh(issue: &IssueUrl, base: &str, branch: &str) -> String {
    format!(
        "/thirdshift:implement {url}\n\
         The base branch is {base}. Review with /thirdshift:code-review using {base} as the fixed point.\n\
         Address the Standards and Spec findings you agree with.\n\
         Push branch {branch} and create a pull request against {base} using /thirdshift:pr, marked ready for review.\n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         Include \"Closes #{number}\" in the PR body.\n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
    )
}

/// The continuation prompt, for a run that picks up an existing Issue branch.
/// With `pr_url`, the agent updates that PR; otherwise it creates one.
pub fn continuation(issue: &IssueUrl, base: &str, branch: &str, pr_url: Option<&str>) -> String {
    let pr = match pr_url {
        Some(url) => format!(
            "Update PR {url} using /thirdshift:pr, rewriting its body to cover the whole branch, marked ready for review."
        ),
        None => format!(
            "Create a pull request against {base} using /thirdshift:pr, marked ready for review."
        ),
    };
    format!(
        "/thirdshift:implement {url}\n\
         \n\
         You are continuing work on branch {branch}, which already has commits (see git log {base}..HEAD). Build on them; don't start over.\n\
         \n\
         The base branch is {base}. Review with /thirdshift:code-review using {base} as the fixed point.\n\
         \n\
         Address the Standards and Spec findings you agree with.\n\
         \n\
         Push branch {branch}.\n\
         \n\
         {pr}\n\
         \n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         \n\
         Include \"Closes #{number}\" in the PR body.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
    )
}

/// The Spec review prompt, for the Spec branch `branch` of `spec` once every
/// Ticket has landed on it, with the draft Spec PR `pr_url` into `base`.
pub fn spec_review(spec: &IssueUrl, base: &str, branch: &str, pr_url: &str) -> String {
    format!(
        "/thirdshift:code-review {base}, with the Spec {url} as the spec\n\
         \n\
         Every Ticket of the Spec {url} has landed on branch {branch}, its Spec branch (see git log {base}..HEAD). Review the Spec as a whole, including how the Tickets' work fits together.\n\
         \n\
         Review with /thirdshift:code-review using {base} as the fixed point and {url} as the spec.\n\
         \n\
         Address the Standards and Spec findings you agree with, using /thirdshift:tdd where it fits, and commit.\n\
         \n\
         Push branch {branch}. Do not rebase or force-push.\n\
         \n\
         Update PR {pr_url} using /thirdshift:pr, rewriting its body to cover the whole Spec. Leave out its Tickets checklist, or keep it between its markers as it is: thirdshift puts it back. Leave the PR a draft: thirdshift marks it ready once you are done.\n\
         \n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         \n\
         Include \"Closes #{number}\" in the PR body.\n\
         \n\
         {HEADLESS}",
        url = spec.url,
        number = spec.number,
    )
}

/// What the last line of an Architecture review's final message starts with,
/// before the URL of the issue it names: the plan it published, the idea it
/// filed instead, or the open issue that already covers that idea. The
/// Architecture review prompt asks for the line, and the Architect run reads
/// it back.
pub const PLAN_LINE: &str = "Architecture review plan: ";
pub const IDEA_LINE: &str = "Architecture review idea: ";
pub const ALREADY_FILED_LINE: &str = "Architecture review already filed: ";

/// The Architecture review prompt, for an Architect run's worktree at the
/// head of the Base branch `base` on origin, pointed at `focus` if the
/// command gave one.
pub fn architecture_review(base: &str, focus: Option<&str>) -> String {
    let focus = match focus {
        Some(focus) => format!("Focus the review on: {focus}\n\n"),
        None => String::new(),
    };
    format!(
        "/thirdshift:improve-codebase-architecture\n\
         \n\
         This is an Architecture review of the base branch {base}: this worktree is checked out at its head on origin, on no branch.\n\
         \n\
         {focus}\
         Find the deepening opportunities with /thirdshift:improve-codebase-architecture, using /thirdshift:codebase-design for the vocabulary. Skip any that an open issue already covers, and take the top recommendation.\n\
         \n\
         If it is Strong, settle its design yourself and publish it as the plan with /thirdshift:to-spec and /thirdshift:to-tickets: a Spec with Tickets, or a single Ticket when one session is enough. Label the plan's top issue, the Spec or the single Ticket, `{NEEDS_TRIAGE}`, not `{READY_FOR_AGENT}`: thirdshift marks it ready once you are done.\n\
         \n\
         If it is not Strong, file it as one issue labelled `{NEEDS_TRIAGE}`, unless an open issue already covers it.\n\
         \n\
         Do not commit or push anything. You may edit files here to check an idea: the worktree is thrown away when you finish. A change the plan needs to CONTEXT.md or an ADR is a Ticket's work, not yours.\n\
         \n\
         End your final message with one of these lines, as its last line, with the issue's full URL and nothing else on the line:\n\
         {PLAN_LINE}<URL of the Spec or the single Ticket you published>\n\
         {IDEA_LINE}<URL of the issue you filed>\n\
         {ALREADY_FILED_LINE}<URL of the open issue that already covers it>\n\
         \n\
         {HEADLESS}",
    )
}

/// The conflict Repair prompt, for a merge of `origin/<merging>` left in
/// progress with conflicts: the Base branch, or in a Merge run the Issue
/// branch with Foreign commits on it.
pub fn conflict_repair(issue: &IssueUrl, merging: &str, branch: &str, pr_url: &str) -> String {
    format!(
        "/thirdshift:resolving-merge-conflicts\n\
         \n\
         A merge of origin/{merging} into {branch} is in progress in this worktree and has conflicts.\n\
         {branch} implements {url}; its pull request is {pr_url}.\n\
         \n\
         Resolve the conflicts, finish the merge, and push {branch}. Do not rebase or force-push.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
    )
}

/// The CI-fix Repair prompt, for red CI on the PR's head commit, listing
/// each of the branch's own failures in `failed` as one to fix, and after
/// them its Inherited failures, if any, as not to fix.
pub fn ci_fix_repair(
    issue: &IssueUrl,
    base: &str,
    branch: &str,
    pr_url: &str,
    failed: &FailedChecks,
) -> String {
    let checks = ci::check_list(&failed.own);
    let inherited = if failed.inherited.is_empty() {
        String::new()
    } else {
        format!(
            "\nAlso failing on `{base}`; don't fix:\n{}",
            ci::check_list(&failed.inherited)
        )
    };
    format!(
        "CI failed on pull request {pr_url} (branch {branch}, implementing {url}).\n\
         \n\
         Failed checks:\n\
         {checks}\
         {inherited}\
         \n\
         Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.\n\
         Run the affected checks locally, commit, and push {branch}.\n\
         \n\
         If a failure is not caused by this branch (it is flaky, or also fails on {base}), do not change code for it. Instead, add it to a \"CI notes\" section of the pull request body with a one-line explanation.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
    )
}

/// The review Repair prompt, for Foreign commits a Merge run has merged into
/// the Issue branch on top of `own_head`, the head it last knew as its own.
pub fn review_repair(issue: &IssueUrl, branch: &str, pr_url: &str, own_head: &str) -> String {
    format!(
        "/thirdshift:code-review {own_head}\n\
         \n\
         Someone else pushed commits to {branch} while it was being worked on, and they have been merged into {branch} in this worktree. {branch} implements {url}; its pull request is {pr_url}. They are merged into the base branch only once you have reviewed them.\n\
         \n\
         Review with /thirdshift:code-review using {own_head} as the fixed point: {branch}'s head before their commits were merged in.\n\
         \n\
         Address the Standards and Spec findings you agree with.\n\
         \n\
         Add each skipped finding to the \"Unaddressed findings\" section of the pull request body, under Standards or Spec, marked as coming from their commits, with at least a one-line reason. Keep the rest of the body as it is.\n\
         \n\
         Commit and push {branch}. Do not rebase or force-push.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
    )
}

/// The resume prompt, for a session whose `killed` background work, by
/// description, was killed when it ended its turn.
pub fn resume(killed: &[&str]) -> String {
    format!(
        "Your background work ({killed}) was killed when your turn ended, because ending the turn ends the session.\n\
         \n\
         Re-run whatever you were waiting on in the foreground, then finish your job. \
         If the re-run hangs or is moved to the background again, stop it with the `TaskStop` tool, by its task id, \
         and say what could not be run, rather than leaving it running.\n\
         \n\
         {HEADLESS}",
        killed = killed.join("; "),
    )
}
