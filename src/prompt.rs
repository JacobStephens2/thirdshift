//! The prompts agent sessions are started with.

use crate::ci::{self, FailedChecks};
use crate::issue::IssueUrl;
use crate::labels::{NEEDS_TRIAGE, READY_FOR_AGENT};

/// Every prompt ends with this, worded for either Harness. A session exits
/// once the agent ends its turn, killing any background task still running.
/// Other sessions on the same machine run the same commands, so a wait on a
/// process name can match theirs and outlast the session's own task.
/// thirdshift can't tell a task the agent gave up on from one it was waiting
/// on, so the agent is to stop the first kind itself, on Claude with
/// `TaskStop`: auto mode may deny it a `kill`.
const HEADLESS: &str = "You run headless: nobody is watching, and ending your turn ends the session. \
    Run tests and other long commands in the foreground, raising the command's timeout if needed. \
    If a command is moved to the background, wait for that task by its own task id or output file, \
    never by process names or patterns (`pgrep`, `ps | grep`, and the like): \
    other sessions on this machine run the same commands. \
    Never end your turn while a background task you depend on is still running: ending the turn kills it. \
    Before ending your turn, stop every background task you no longer need, by its task id (with the `TaskStop` tool, if you have it): \
    a task still running when your turn ends is taken as work you were waiting on.\n";

/// Sessions replaces this placeholder with its git-ignored report directory.
pub const REVIEW_REPORTS_DIRECTORY: &str = "<review reports directory>";

/// The same report contract for every session that runs the code review.
pub const REVIEW_REPORTS: &str = "Write the reviewers' reports, with each axis's final files-read list, \
    to `<review reports directory>/standards.md` (Standards) and `<review reports directory>/spec.md` (Spec).";

/// The shared review instruction, with each Session prompt's variations.
enum ReviewInstruction {
    Implement,
    SpecReview,
    ForeignCommits,
}

impl ReviewInstruction {
    fn address_findings(&self) -> String {
        let ending = match self {
            Self::SpecReview => ", using the `thirdshift-tdd` skill where it fits, and commit.",
            _ => ".",
        };
        format!(
            "For each Standards or Spec finding that says behaviour is wrong, run its test or command as written before deciding the finding. \
             If it fails as the finding says, fix the code and keep the test. \
             If it passes, you may decline the finding, citing the run. \
             A passing run counts only when it exercises that finding; a green suite does not. \
             Address the Standards and Spec findings you agree with{ending}\n\
             {REVIEW_REPORTS}"
        )
    }

    fn unaddressed_findings(&self) -> String {
        let (opening, listing, source, ending) = match self {
            Self::ForeignCommits => (
                "Add each skipped finding to the",
                "of the pull request body,",
                " marked as coming from their commits,",
                " Keep the rest of the body as it is.",
            ),
            _ => (
                "In the PR body, add an",
                "listing each skipped finding",
                "",
                "",
            ),
        };
        format!(
            "{opening} \"Unaddressed findings\" section {listing} under Standards or Spec,{source} \
             with the line of code, plan decision, ADR or run that refutes each finding. \
             If you decline a finding because a Spec or the Day shift must decide, \
             file a `needs-triage` issue unless an open issue already covers it, and link the issue from the entry. \
             The issue must say what the finding is, its evidence, which pull request raised it, and why the call is not yours to make. \
             Give a new issue only the `needs-triage` label so it pauses nothing. \
             Name in the pull request body any changed file the review left unread.{ending}"
        )
    }
}

/// The fresh prompt, for a run that starts a new Issue branch.
pub fn fresh(issue: &IssueUrl, base: &str, branch: &str) -> String {
    let review = ReviewInstruction::Implement;
    format!(
        "/thirdshift-implement {url}\n\
         The base branch is {base}. Review with the `thirdshift-code-review` skill using {base} as the fixed point.\n\
         {address_findings}\n\
         Push branch {branch} and create a pull request against {base} using the `thirdshift-pr` skill, marked ready for review.\n\
         {unaddressed_findings}\n\
         Include \"Closes #{number}\" in the PR body.\n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
        address_findings = review.address_findings(),
        unaddressed_findings = review.unaddressed_findings(),
    )
}

/// The continuation prompt, for a run that picks up an existing Issue branch.
/// With `pr_url`, the agent updates that PR; otherwise it creates one.
pub fn continuation(issue: &IssueUrl, base: &str, branch: &str, pr_url: Option<&str>) -> String {
    let review = ReviewInstruction::Implement;
    let pr = match pr_url {
        Some(url) => format!(
            "Update PR {url} using the `thirdshift-pr` skill, rewriting its body to cover the whole branch, marked ready for review."
        ),
        None => format!(
            "Create a pull request against {base} using the `thirdshift-pr` skill, marked ready for review."
        ),
    };
    format!(
        "/thirdshift-implement {url}\n\
         \n\
         You are continuing work on branch {branch}, which already has commits (see git log {base}..HEAD). Build on them; don't start over.\n\
         \n\
         The base branch is {base}. Review with the `thirdshift-code-review` skill using {base} as the fixed point.\n\
         \n\
         {address_findings}\n\
         \n\
         Push branch {branch}.\n\
         \n\
         {pr}\n\
         \n\
         {unaddressed_findings}\n\
         \n\
         Include \"Closes #{number}\" in the PR body.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
        number = issue.number,
        address_findings = review.address_findings(),
        unaddressed_findings = review.unaddressed_findings(),
    )
}

/// The Spec review prompt, for the Spec branch `branch` of `spec` once every
/// Ticket has landed on it, with the draft Spec PR `pr_url` into `base`.
pub fn spec_review(spec: &IssueUrl, base: &str, branch: &str, pr_url: &str) -> String {
    let review = ReviewInstruction::SpecReview;
    format!(
        "/thirdshift-code-review {base}, with the Spec {url} as the spec\n\
         \n\
         Every Ticket of the Spec {url} has landed on branch {branch}, its Spec branch (see git log {base}..HEAD). Review the Spec as a whole, including how the Tickets' work fits together.\n\
         \n\
         Review with the `thirdshift-code-review` skill using {base} as the fixed point and {url} as the spec.\n\
         \n\
         {address_findings}\n\
         \n\
         Push branch {branch}. Do not rebase or force-push.\n\
         \n\
         Update PR {pr_url} using the `thirdshift-pr` skill, rewriting its body to cover the whole Spec. Leave out its Tickets checklist, or keep it between its markers as it is: thirdshift puts it back. Leave the PR a draft: thirdshift marks it ready once you are done.\n\
         \n\
         {unaddressed_findings}\n\
         \n\
         Include \"Closes #{number}\" in the PR body.\n\
         \n\
         {HEADLESS}",
        url = spec.url,
        number = spec.number,
        address_findings = review.address_findings(),
        unaddressed_findings = review.unaddressed_findings(),
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
        "/thirdshift-improve-codebase-architecture\n\
         \n\
         This is an Architecture review of the base branch {base}: this worktree is checked out at its head on origin, on no branch.\n\
         \n\
         {focus}\
         Find the deepening opportunities with the `thirdshift-improve-codebase-architecture` skill, using the `thirdshift-codebase-design` skill for the vocabulary. Skip any that an open issue already covers, and take the top recommendation.\n\
         \n\
         If it is Strong, settle its design yourself and publish it as the plan with the `thirdshift-to-spec` and `thirdshift-to-tickets` skills: a Spec with Tickets, or a single Ticket when one session is enough. Label the plan's top issue, the Spec or the single Ticket, `{NEEDS_TRIAGE}`, not `{READY_FOR_AGENT}`: thirdshift marks it ready once you are done.\n\
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
        "/thirdshift-resolving-merge-conflicts\n\
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
    let review = ReviewInstruction::ForeignCommits;
    format!(
        "/thirdshift-code-review {own_head}\n\
         \n\
         Someone else pushed commits to {branch} while it was being worked on, and they have been merged into {branch} in this worktree. {branch} implements {url}; its pull request is {pr_url}. They are merged into the base branch only once you have reviewed them.\n\
         \n\
         Review with the `thirdshift-code-review` skill using {own_head} as the fixed point: {branch}'s head before their commits were merged in.\n\
         \n\
         {address_findings}\n\
         \n\
         {unaddressed_findings}\n\
         \n\
         Commit and push {branch}. Do not rebase or force-push.\n\
         \n\
         {HEADLESS}",
        url = issue.url,
        address_findings = review.address_findings(),
        unaddressed_findings = review.unaddressed_findings(),
    )
}

/// The resume prompt, for a session whose `killed` background work, by
/// description, was killed when it ended its turn.
pub fn resume(killed: &[&str]) -> String {
    format!(
        "Your background work ({killed}) was killed when your turn ended, because ending the turn ends the session.\n\
         \n\
         Re-run whatever you were waiting on in the foreground, then finish your job. \
         If the re-run hangs or is moved to the background again, stop it by its task id (with the `TaskStop` tool, if you have it), \
         and say what could not be run, rather than leaving it running.\n\
         \n\
         {HEADLESS}",
        killed = killed.join("; "),
    )
}

/// The Security audit's terminal protocol: only these final lines are read.
pub const AUDIT_COMPLETE_LINE: &str = "Security audit: complete";
pub const AUDIT_INCOMPLETE_LINE: &str = "Security audit: incomplete";

pub fn security_audit(
    base: &str,
    commit: &str,
    root: &std::path::Path,
    output: &std::path::Path,
    threat_model: Option<&str>,
) -> String {
    let threat_model = threat_model
        .map(|file| format!("Read the repository's threat-model document `{file}`.\n"))
        .unwrap_or_default();
    format!(
        "/thirdshift-security-audit\nUse the `thirdshift-security-audit` skill in full audit mode with report artifacts and the `quick` profile.\nAudit the whole repository at commit `{commit}`, the head of Base branch `{base}` on origin. All vendored and third-party code are out of scope.\n{threat_model}Output directory: `{output}`.\nAudit root: `{root}`.\nRead compatible earlier runs under the audit root before planning this audit.\nThis session commits, pushes, opens and publishes nothing, changes no repository source, and touches no deployed site. It reproduces and fixes nothing.\nIf a decision or missing prerequisite would require asking, record run_status incomplete with the reason, and end incomplete rather than asking.\nWrite the skill's report artifacts and run-metadata.json; mark run_status complete only when all required artifacts are written. Run both skill validators before finishing.\nEnd your final message with exactly `{AUDIT_COMPLETE_LINE}` after writing valid artifacts, or `{AUDIT_INCOMPLETE_LINE}` when incomplete.\n\n{HEADLESS}",
        output = output.display(),
        root = root.display()
    )
}

/// One guidance session on the Run's change, before Delivery pushes and
/// enters the Repair loop. The separate Security audit remains a full audit.
pub const SECURITY_REVIEW_REPORT_FILE: &str = "<private report file>";
pub const SECURITY_REVIEW_MERGE_BASE: &str = "<merge base commit>";

pub fn security_review(issue: &IssueUrl, base: &str, branch: &str) -> String {
    format!(
        "Use the `thirdshift-security-audit` skill in guidance mode.\n\
         Review the change for {url} on branch {branch} against Base branch {base} with `git diff {SECURITY_REVIEW_MERGE_BASE}...HEAD`, including supporting code.\n\
         Read the relevant security attack-class guidance and the repository's SECURITY.md or threat model when present.\n\
         Use one session. Do not delegate auditors or run the full six-phase audit, validators, coverage ledger or audit artifacts.\n\
         Fix only a Security finding you can show with a failing proof-of-concept test; run it before fixing, keep it as a regression test, and rerun it afterward. Use harmless local payloads; touch no deployed site or real third-party service. Commit each fix.\n\
         Merge base commit: `{SECURITY_REVIEW_MERGE_BASE}`.\n\
         Classify introduced findings against this pinned merge base: run each proof-of-concept on HEAD and the untouched merge base; a test that also fails there is pre-existing. Never include pre-existing vulnerability details, titles, fingerprints, tests or private-record links in the pull request or final message, and do not fix those here.\n\
         Private report file: `{SECURITY_REVIEW_REPORT_FILE}`.\n\
         Write a JSON array to that file, even when empty ([]). Each entry is an old finding with exactly these fields: fingerprint (stable across reviews and audits, no newlines or backticks), title (one line), description (the private write-up and evidence), proof_of_concept (test: full test text, command: exact command run at both commits, head_exit_code: positive failing exit code, merge_base_exit_code: positive failing exit code, notes: reproduction evidence and scoring rationale, severity: critical/high/medium/low/informational scored with the skill's likelihood-and-impact rubric, fix_size: single/spec judged from whether one session can hold the fix). All text fields are nonempty. Do not commit or copy this report into the worktree. thirdshift records it privately with the Security run's fingerprint matching and reproduced-outcome format so a later Security run can fix it when allowed; existing records and the Day shift's grades are preserved. Fixing old findings belongs to a separate Security run.\n\
         Update this branch's pull request: list each unaddressed introduced finding under Security in its Unaddressed findings, with evidence and why it is unaddressed. Preserve Standards and Spec entries and the rest of the body. Do not merge. Delivery pushes any new commits and marks the PR ready after this session.\n\
         If refused, incomplete, or missing a prerequisite, say so and do not claim completion.\n\
         After a complete review, end your final message with exactly one line of the form:\n\
         Security review: {{\"unaddressed_count\":0,\"findings\":[],\"pre_existing_count\":0}}\n\
         unaddressed_count counts only unaddressed findings introduced by this change; findings is an array of their short titles, with one title per finding. pre_existing_count is the number of entries in the private report. No old-finding details belong in this line.\n\n\
         {HEADLESS}",
        url = issue.url,
    )
}

pub fn security_reproduction(commit: &str, finding: &str, test: &std::path::Path) -> String {
    format!(
        "Reproduce this recorded Security finding at audited commit `{commit}`.\n\
         Read the `thirdshift-security-audit` skill's likelihood-and-impact severity rubric.\n\
         Write a proof-of-concept test from the finding's validation plan, then run it against the untouched code in this throwaway worktree. Use harmless payloads only, never a deployed site or a real third-party service. Do not fix or change repository source.\n\
         If the test reproduces the finding, score its severity with the skill's rubric and judge whether one session can hold the fix (single) or it needs a Spec (spec).\n\
         Test file: `{test}`.\n\
         Write the test's full text to this file, even when not reproduced. In your final message, give reproduction notes: the exact command, its result, and, when reproduced, likelihood, impact and the fix-size reasoning.\n\
         End your final message with exactly `Security reproduction: reproduced <severity> <size>`, where severity is critical, high, medium, low or informational and size is single or spec, or `Security reproduction: not reproduced`.\n\
         This session commits, pushes, opens and publishes nothing.\n\n\
         Recorded finding:\n{finding}\n\n{HEADLESS}",
        test = test.display(),
    )
}

pub const SECURITY_FIX_LINE: &str = "Security fix Ticket: ";
pub const SECURITY_FIX_SPEC_LINE: &str = "Security fix Spec: ";

pub fn security_fix(base: &str, url: &str, finding: &str) -> String {
    format!(
        "Publish the fix for this reproduced Security finding on Base branch `{base}`.\n\
         Read the private record at {url} and the record below.\n\
         Follow the completed reproduction's fix size: single publishes one Ticket; spec publishes a Spec with Tickets using `thirdshift-to-spec` and `thirdshift-to-tickets`.\n\
         On a public repository, publish one new top issue labelled `needs-triage`. Create that label if missing. On a private repository, reuse the finding's issue as the top issue and preserve its body and evidence; add the bigger fix's Tickets as its native sub-issues.\n\
         Every new issue is terse: it says only what the fix changes and links the private record. It carries none of the write-up, trace, evidence, reproduction notes, proof-of-concept test or exploit details, even paraphrased. Keep those in the private record. This overrides the skills' templates.\n\
         A Spec's Tickets must be new, open, labelled `ready-for-agent`, linked as native sub-issues with their native blocking links, and have no sub-issues of their own. Read the links back before finishing.\n\
         This session changes no repository source, commits and pushes nothing, opens no pull request, and does not implement the fix. thirdshift checks every issue, marks the top issue ready, labels all fix issues security-fix and dispatches its Run or Spec run.\n\
         End your final message with exactly `{SECURITY_FIX_LINE}<Issue URL>` for single or `{SECURITY_FIX_SPEC_LINE}<Issue URL>` for spec, naming the top issue.\n\n\
         Private record:\n{finding}\n\n{HEADLESS}"
    )
}
