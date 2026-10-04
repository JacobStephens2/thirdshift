//! Delivery: what a Run, or a Spec run for its Spec PR, does from its
//! worktree once its work begins. The opening session, the push after it,
//! marking the pull request ready, the Repair loop that keeps it mergeable
//! and green, the Self-merge in a Merge run, and the Failed run path when any
//! of these fails.

use anyhow::{Context, Result, bail};

use crate::base_fix::BaseFix;
use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{self, FailedRun, PolicyRefusal};
use crate::github::{self, Mergeable, PullRequest};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::poll;
use crate::progress;
use crate::prompt;
use crate::run::{Goal, Reached};
use crate::session::{Logs, Sessions};
use crate::worktree::{Merge, Worktree};

/// A Delivery of the pull request for `issue`, the Run's issue or the Spec,
/// into the Base branch `base`, to `goal`.
pub struct Delivery<'a> {
    pub issue: &'a IssueUrl,
    pub base: &'a str,
    pub goal: Goal,
    /// The one Base fix the Delivery may start, or wait on, when its only red
    /// checks are Inherited failures.
    pub base_fix: &'a mut BaseFix,
    /// Where its Session logs go.
    pub logs: &'a Logs,
}

/// The session a Delivery opens with: the implement session, or the Spec
/// review.
pub struct Opening<'a> {
    /// The session's kind, in its progress lines and log name.
    pub kind: &'a str,
    pub prompt: String,
    /// Whether the branch is first fast-forwarded to its head on origin, as
    /// the Spec branch is to the Tickets landed there.
    pub catch_up_from_origin: bool,
}

impl Delivery<'_> {
    /// Take the pull request from the branch checked out in `worktree` to the
    /// goal. With `opening.catch_up_from_origin`, first fast-forward the
    /// branch to origin. Then run the opening session, push the branch, for
    /// any commit the session left unpushed, run `before_ready`, and mark the
    /// pull request ready, failing unless it exists, is open and targets the
    /// Base branch. Then keep it mergeable and its CI green through the
    /// Repair loop, and for [`Goal::Merged`], Self-merge it. A merge that
    /// fails goes back round the Repair loop and is tried again on the new
    /// head; if that round finds nothing to fix, this fails with a
    /// `PolicyRefusal`. Any failure goes through the Failed run path, which
    /// keeps the worktree if its work did not reach origin; otherwise it is
    /// cleaned up when this returns.
    pub fn deliver(
        mut self,
        worktree: Worktree,
        opening: Opening,
        before_ready: impl FnOnce() -> Result<()>,
    ) -> Result<Reached, FailedRun> {
        let caught_up = if opening.catch_up_from_origin {
            worktree.fast_forward_to_origin()
        } else {
            Ok(())
        };
        let (delivered, log) = match caught_up {
            Ok(()) => Sessions::within(self.logs, worktree.path(), |sessions| {
                sessions.run(opening.kind, &opening.prompt)?;
                worktree.push()?;
                before_ready()?;
                let pr = mark_pr_ready(self.issue, worktree.branch(), self.base)?;
                self.take_to_goal(sessions, &worktree, &pr)?;
                Ok(pr.url)
            }),
            Err(error) => (Err(error), None),
        };
        match delivered {
            Ok(pr_url) => Ok(Reached {
                pr_url,
                goal: self.goal,
                log,
                ticket_lines: Vec::new(),
            }),
            Err(error) => Err(failed_run::fail(
                self.issue, worktree, self.base, log, error,
            )),
        }
    }

    /// Take the ready PR `pr`, from the branch checked out in `worktree`, to
    /// the goal, as [`Delivery::deliver`] says, starting each Repair through
    /// `sessions`.
    fn take_to_goal(
        &mut self,
        sessions: &Sessions,
        worktree: &Worktree,
        pr: &PullRequest,
    ) -> Result<()> {
        let (issue, base, goal) = (self.issue, self.base, self.goal);
        let branch = worktree.branch();
        let mut repair_loop = RepairLoop {
            issue,
            worktree,
            base,
            pr_url: &pr.url,
            goal,
            budgets: Budgets::default(),
            base_fix: self.base_fix,
            sessions,
        };
        let mut watched = repair_loop.run()?;
        loop {
            ensure_pr_ready_and_mergeable(issue, branch)?;
            if interrupt::requested() {
                bail!("interrupted");
            }
            if goal == Goal::ReadyForReview {
                return Ok(());
            }
            progress::step(format_args!("merging the PR into {base}"));
            let Err(error) = github::merge(issue, branch, &watched) else {
                after_merge(issue, worktree, pr);
                return Ok(());
            };
            progress::step(format_args!("the merge failed: {error:#}"));
            match repair_loop.round_after_failed_merge(&watched)? {
                Round::NewHead(head) => watched = head,
                Round::NothingToFix => {
                    // Only a PR still ready and mergeable is left ready.
                    ensure_pr_ready_and_mergeable(issue, branch)?;
                    return Err(error.context(PolicyRefusal));
                }
            }
        }
    }
}

/// The Self-merge's steps after the merge: delete the Issue branch on origin,
/// and close the issue unless it is closed already. GitHub may close it too, a
/// moment after the merge, or may not, so thirdshift does not wait to see.
/// The merge can't be undone, so these never fail the Run, and an interrupt no
/// longer stops it: a step that fails is a warning naming the fix to make by
/// hand.
fn after_merge(issue: &IssueUrl, worktree: &Worktree, pr: &PullRequest) {
    let branch = worktree.branch();
    if let Err(error) = interrupt::retry_if_interrupted(|| worktree.delete_from_origin()) {
        progress::warn(
            &error,
            format_args!(
                "could not delete {branch} on origin, so delete it by hand: \
                 git push origin --delete {branch}"
            ),
        );
    }
    let comment = format!(
        "Closed by #{}, merged into {} by a thirdshift Merge run.",
        pr.number, pr.base
    );
    if let Err(error) = interrupt::retry_if_interrupted(|| close_unless_closed(issue, &comment)) {
        progress::warn(
            &error,
            format_args!(
                "could not close issue #{number}, so if it is still open, close it by hand: \
                 gh issue close {number} --repo {repo} --comment '{quoted}'",
                number = issue.number,
                repo = issue.repo_slug(),
                quoted = comment.replace('\'', r"'\''")
            ),
        );
    }
}

/// Close `issue` with `comment`, unless it is closed already.
fn close_unless_closed(issue: &IssueUrl, comment: &str) -> Result<()> {
    if !github::issue_is_open(issue)? {
        return Ok(());
    }
    progress::step(format_args!("closing issue #{}", issue.number));
    github::close_issue(issue, comment)
}

/// The most Repair sessions a Delivery starts, conflict, CI-fix and review
/// combined.
const MAX_REPAIRS: usize = 5;

/// The most times a Delivery goes round again because the Base branch moved while
/// CI ran or since a merge was tried, or, in a Merge run, because Foreign
/// commits arrived, whether or not the merge that follows needs a Repair. A
/// clean merge of the Base branch uses no Repair, so without this a busy Base
/// branch could keep a Delivery going forever.
const MAX_UPSTREAM_MOVES: usize = 5;

/// What a Delivery has spent of its Repair and upstream-move budgets,
/// across every round of the Repair loop, those after a failed merge
/// included.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Budgets {
    repairs: usize,
    upstream_moves: usize,
}

impl Budgets {
    /// Counts the Repair about to start, as `repair-<n>`, or fails if it would
    /// be one too many.
    fn next_repair(&mut self, cause: &str) -> Result<String> {
        if self.repairs == MAX_REPAIRS {
            bail!("repairs exhausted: {cause}");
        }
        self.repairs += 1;
        progress::step(format_args!(
            "{cause}; starting Repair {} of {MAX_REPAIRS}",
            self.repairs
        ));
        Ok(format!("repair-{}", self.repairs))
    }

    /// Counts a round taken because `upstream` moved, saying `why`, or fails
    /// if it would be one too many.
    fn count_upstream_move(&mut self, upstream: &str, why: std::fmt::Arguments) -> Result<()> {
        if self.upstream_moves == MAX_UPSTREAM_MOVES {
            bail!("{upstream} kept moving: merged it again {MAX_UPSTREAM_MOVES} times");
        }
        self.upstream_moves += 1;
        progress::step(why);
        Ok(())
    }

    /// Counts a round taken because `origin/<base>` moved `when`.
    fn count_base_move(&mut self, base: &str, when: &str) -> Result<()> {
        self.count_upstream_move(
            &format!("origin/{base}"),
            format_args!("origin/{base} moved {when}; merging it again"),
        )
    }
}

/// What a round of the Repair loop after a failed merge came to.
enum Round {
    /// A head whose CI was found green or absent, to try the merge on.
    NewHead(String),
    /// The same head, with no Repair and no upstream move: a policy refusal.
    NothingToFix,
}

/// Keeps the PR mergeable and its CI green, within the Delivery's budgets.
struct RepairLoop<'a> {
    issue: &'a IssueUrl,
    worktree: &'a Worktree,
    base: &'a str,
    pr_url: &'a str,
    goal: Goal,
    budgets: Budgets,
    /// The Delivery's one Base fix, kept apart from its budgets.
    base_fix: &'a mut BaseFix,
    /// Where each Repair session runs.
    sessions: &'a Sessions<'a>,
}

impl RepairLoop<'_> {
    /// In a Merge run, first take in any Foreign commits. Then merge the
    /// Base branch (never rebase), push, and watch CI on the head commit,
    /// starting a Repair session for a conflict or red
    /// CI and then going round again, since the Base branch may have moved
    /// meanwhile. Green or absent CI also goes round again if the Base branch
    /// moved while CI ran, or, in a Merge run, if Foreign commits arrived.
    /// A red check that also fails on the Base branch commit the head last
    /// merged in is an Inherited failure, which no Repair is started for: the
    /// CI-fix Repair is given the branch's own failures, and when there are
    /// none, the loop goes round again if the Base branch moved since, counted
    /// as a Base move, and otherwise once its Base fix has merged, or fails
    /// naming the checks and the Base branch commit: see [`BaseFix::fix`]. A
    /// Base fix's own Run sees no Inherited failures.
    /// If, after a CI-fix Repair and the Base branch merged again, the head
    /// is still the one whose CI failed, it gets its Check re-run instead of
    /// a watch, whatever the Repair concluded: see [`ci::rerun`]. CI then
    /// green, or red only on Inherited failures, is taken as from any watch.
    /// Returns the head commit whose CI was last watched and found green or
    /// absent. Fails with a Declined CI fix if that Check re-run leaves a
    /// check of the branch's own red, or there can be none, and once a Repair
    /// beyond `MAX_REPAIRS`, or a round beyond `MAX_UPSTREAM_MOVES`, would be
    /// needed.
    fn run(&mut self) -> Result<String> {
        let (issue, worktree, base, pr_url) = (self.issue, self.worktree, self.base, self.pr_url);
        let branch = worktree.branch();
        // The head the last CI-fix Repair was given, with its failed checks.
        let mut handed_to_repair: Option<(String, FailedChecks)> = None;
        loop {
            if self.goal == Goal::Merged {
                self.take_in_foreign_commits()?;
            }
            if let Merge::Conflicted(pending) = worktree.merge_base_branch(base)? {
                let kind = self.budgets.next_repair("conflict")?;
                self.sessions
                    .run(&kind, &prompt::conflict_repair(issue, base, branch, pr_url))?;
                worktree.ensure_merged(&pending)?;
                continue;
            }
            worktree.push()?;
            let head = worktree.head()?;
            let base_commit = worktree.merged_base_commit(base)?;
            let compared_with = self
                .base_fix
                .sees_inherited_failures()
                .then_some(base_commit.as_str());
            let unchanged = handed_to_repair
                .take()
                .filter(|(handed, _)| *handed == head);
            let ci = match unchanged {
                None => ci::watch(issue, &head, compared_with)?,
                // A Declined CI fix, unless the head's one Check re-run turns
                // the branch's own checks green: it gets no second Repair.
                Some((_, failed)) => match ci::rerun(issue, &head, compared_with, &failed)? {
                    Some(ci) if !ci.has_own_failures() => ci,
                    _ => bail!(
                        "CI red on {} and the Repair found nothing to fix on the branch",
                        ci::short(&head)
                    ),
                },
            };
            match ci {
                Ci::Absent | Ci::Passed => {
                    if worktree.base_branch_moved(base)? {
                        self.budgets.count_base_move(base, "while CI ran")?;
                    } else if self.goal == Goal::ReadyForReview
                        || worktree.new_commits_on_origin()?.is_empty()
                    {
                        return Ok(head);
                    }
                }
                Ci::Failed(failed) => {
                    if !failed.inherited.is_empty() {
                        progress::step(format_args!(
                            "Inherited failures (also failing on {base} at {}): {}",
                            ci::short(&base_commit),
                            ci::check_names(&failed.inherited)
                        ));
                    }
                    if failed.own.is_empty() {
                        // Someone may have fixed the Base branch since.
                        if worktree.base_branch_moved(base)? {
                            self.budgets.count_base_move(base, "while CI ran")?;
                            continue;
                        }
                        self.base_fix.fix(
                            worktree.launch(),
                            issue,
                            pr_url,
                            base,
                            &base_commit,
                            &failed.inherited,
                        )?;
                        continue;
                    }
                    let kind = self.budgets.next_repair("CI red")?;
                    self.sessions.run(
                        &kind,
                        &prompt::ci_fix_repair(issue, base, branch, pr_url, &failed),
                    )?;
                    handed_to_repair = Some((head, failed));
                }
            }
        }
    }

    /// Fetch the Issue branch from origin and merge in any Foreign commits on
    /// it (never rebase), counting that as a round and handing a conflict to
    /// a conflict Repair. Then a review Repair reviews them from the head the
    /// Run last knew as its own, the local head before the merge. Goes round
    /// again until origin has nothing new, since more may land during either
    /// Repair, and the push after them would be rejected.
    fn take_in_foreign_commits(&mut self) -> Result<()> {
        let (issue, worktree, pr_url) = (self.issue, self.worktree, self.pr_url);
        let branch = worktree.branch();
        let upstream = worktree.upstream();
        loop {
            let foreign = worktree.new_commits_on_origin()?;
            if foreign.is_empty() {
                return Ok(());
            }
            let own_head = worktree.head()?;
            self.budgets.count_upstream_move(
                &upstream,
                format_args!("{upstream} has new commits; merging them in"),
            )?;
            for sha in &foreign {
                progress::step(format_args!("merging new commit {sha} from {upstream}"));
            }
            if let Merge::Conflicted(pending) = worktree.merge_new_commits()? {
                let kind = self
                    .budgets
                    .next_repair(&format!("conflict with new commits on {upstream}"))?;
                self.sessions.run(
                    &kind,
                    &prompt::conflict_repair(issue, branch, branch, pr_url),
                )?;
                worktree.ensure_merged(&pending)?;
            }
            let kind = self
                .budgets
                .next_repair(&format!("new commits on {upstream} to review"))?;
            self.sessions.run(
                &kind,
                &prompt::review_repair(issue, branch, pr_url, &own_head),
            )?;
        }
    }

    /// Go round again after a merge of `watched` failed, counting a Base
    /// branch that moved since as an upstream move. GitHub's error text is never
    /// consulted.
    fn round_after_failed_merge(&mut self, watched: &str) -> Result<Round> {
        let before = self.budgets;
        if self.worktree.base_branch_moved(self.base)? {
            self.budgets
                .count_base_move(self.base, "since the merge was tried")?;
        }
        let head = self.run()?;
        Ok(if head == watched && self.budgets == before {
            Round::NothingToFix
        } else {
            Round::NewHead(head)
        })
    }
}

/// Mark the PR whose head is `branch` ready for review, failing unless it
/// exists, is open and targets `base`.
fn mark_pr_ready(issue: &IssueUrl, branch: &str, base: &str) -> Result<PullRequest> {
    progress::step("checking the PR");
    let pr = open_pr(issue, branch)?;
    if pr.base != base {
        bail!("PR targets {}, not {base}", pr.base);
    }
    if pr.is_draft {
        github::mark_ready(issue, branch)?;
    }
    Ok(pr)
}

/// The PR whose head is `branch`, failing unless it exists and is open.
fn open_pr(issue: &IssueUrl, branch: &str) -> Result<PullRequest> {
    let pr = github::pull_request_for(issue, branch)?.context("no PR found")?;
    if !pr.is_open() {
        bail!("PR {} is {}, not open", pr.url, pr.state);
    }
    Ok(pr)
}

/// Fail unless the PR for `branch` is still open, ready for review, and
/// mergeable, waiting up to the grace period for GitHub to work out the last.
fn ensure_pr_ready_and_mergeable(issue: &IssueUrl, branch: &str) -> Result<()> {
    progress::step("checking the PR is open, ready and mergeable");
    let pr = open_pr(issue, branch)?;
    if pr.is_draft {
        bail!("PR {} is a draft", pr.url);
    }
    let mergeable = poll::within(poll::grace_period(), || {
        Ok(Some(github::mergeable(issue, branch)?).filter(|m| *m != Mergeable::Unknown))
    })?;
    match mergeable {
        Some(Mergeable::Yes) => Ok(()),
        Some(Mergeable::No) => bail!("PR {} is not mergeable", pr.url),
        _ => bail!(
            "GitHub has not worked out whether PR {} is mergeable",
            pr.url
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_of_every_kind_share_one_cap_and_the_one_too_many_names_its_cause() {
        let mut budgets = Budgets::default();
        for n in 1..=MAX_REPAIRS {
            let cause = if n % 2 == 0 { "conflict" } else { "CI red" };
            assert_eq!(budgets.next_repair(cause).unwrap(), format!("repair-{n}"));
        }

        for cause in ["new commits on origin/issue-7 to review", "conflict"] {
            let error = budgets.next_repair(cause).unwrap_err();
            assert_eq!(error.to_string(), format!("repairs exhausted: {cause}"));
        }
        assert_eq!(budgets.repairs, MAX_REPAIRS);
    }

    #[test]
    fn upstream_moves_of_the_base_or_the_issue_branch_share_one_budget() {
        let mut budgets = Budgets::default();
        for _ in 1..MAX_UPSTREAM_MOVES {
            budgets.count_base_move("main", "while CI ran").unwrap();
        }
        budgets
            .count_upstream_move("origin/issue-7", format_args!("new commits"))
            .unwrap();

        let error = budgets.count_base_move("main", "while CI ran").unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/main kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
        let error = budgets
            .count_upstream_move("origin/issue-7", format_args!("new commits"))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/issue-7 kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
    }

    #[test]
    fn the_repair_cap_and_the_upstream_move_budget_are_spent_apart() {
        let mut budgets = Budgets::default();
        for _ in 0..MAX_REPAIRS {
            budgets.next_repair("CI red").unwrap();
        }

        for _ in 0..MAX_UPSTREAM_MOVES {
            budgets.count_base_move("main", "while CI ran").unwrap();
        }
        assert_eq!(
            budgets,
            Budgets {
                repairs: MAX_REPAIRS,
                upstream_moves: MAX_UPSTREAM_MOVES
            }
        );
    }
}
