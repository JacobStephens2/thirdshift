//! The Repair loop: what keeps a Delivery's ready pull request mergeable
//! and its CI green, within its Repair and upstream-move budgets, and, in a
//! Merge run, merges it, going round again after a failed merge until it
//! merges or is a Policy refusal. It reaches the Issue branch, CI, the
//! Repair sessions, the Base fix and the pull request only through
//! [`Outside`].

use anyhow::{Result, bail};

use crate::pull_request::MergeAttempt;

use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::{MergeHold, PolicyRefusal};
use crate::run::Goal;
use crate::worktree::{ForeignCommits, Merge};

use super::Stage;
use crate::session::Kind;

/// Which upstream a conflict Repair resolves a merge of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Upstream {
    /// The Base branch on origin.
    BaseBranch,
    /// The Issue branch on origin, with Foreign commits on it.
    IssueBranch,
}

/// A Repair, by what it is.
pub(super) enum Repair<'a> {
    /// Resolve a conflicted merge of this upstream.
    Conflict(Upstream),
    /// Fix the branch's own failures among these failed checks.
    CiFix(&'a FailedChecks),
    /// Review the Foreign commits merged in on top of this commit, the head
    /// the Run last knew as its own.
    Review { from: &'a str },
}

/// What the Repair loop does or reads outside itself.
pub(super) trait Outside {
    /// A merge left in progress for a conflict Repair.
    type Pending;

    /// Enter `stage`, before the operation or budget gate it names.
    fn stage(&mut self, stage: Stage);

    /// Merge the Base branch, fetched from origin, into the Issue branch,
    /// returning the selected commit or the merge left pending if it conflicted.
    fn merge_base_branch(&mut self) -> Result<Merge<Self::Pending>>;
    /// Consume a Foreign commit observation, merging its sampled origin head.
    fn merge_new_commits(&mut self, observed: ForeignCommits) -> Result<Merge<Self::Pending>>;
    /// Fail unless the merge `pending` was finished.
    fn ensure_merged(&mut self, pending: &Self::Pending) -> Result<()>;
    /// Push the Issue branch.
    fn push(&mut self) -> Result<()>;
    /// The Issue branch's head commit.
    fn head(&mut self) -> Result<String>;
    /// Whether the current committed tree equals the tree at `commit`.
    /// Failure cannot establish that review may be skipped.
    fn same_tree_as(&mut self, commit: &str) -> Result<bool>;
    /// Whether the Base branch on origin has commits the Issue branch has
    /// not merged yet.
    fn base_branch_moved(&mut self) -> Result<bool>;
    /// Observe the sampled origin and local heads and the oldest-first
    /// Foreign commits between them, without merging.
    fn new_commits_on_origin(&mut self) -> Result<ForeignCommits>;

    /// Assess CI on `head` against the selected Base commit, remembering
    /// re-run allowances for this Delivery inside CI watch.
    fn assess_ci(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci>;
    /// Tell CI watch that the CI-fix Repair given these failures on this
    /// full head completed, including its pull request postcheck.
    fn ci_fix_completed(&mut self, head: String, failed: FailedChecks);

    /// Start the Repair session of `kind`, as in `Kind::Repair(1)`, for
    /// `repair`.
    fn repair(&mut self, kind: Kind, repair: Repair) -> Result<()>;

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    fn sees_inherited_failures(&mut self) -> bool;
    /// Take the Base fix for the Inherited failures in `failed`, which also
    /// fail on the Base branch at `base_commit`, returning once the Run is to
    /// go round again.
    fn base_fix(&mut self, base_commit: &str, failed: &FailedChecks) -> Result<()>;

    /// Whether GitHub reports the open, ready pull request mergeable with the
    /// Base branch. Mergeability it has not worked out in time is not.
    fn pr_mergeable(&mut self) -> Result<bool>;
    /// Fail unless the pull request is open, ready for review and mergeable.
    fn ensure_pr_ready_and_mergeable(&mut self) -> Result<()>;
    /// Merge the pull request at `head`.
    fn merge(&mut self, head: &str) -> Result<MergeAttempt>;
    /// A reason to hold Self-merge, checked on the head about to be merged.
    /// None lets the merge proceed. The default holds nothing.
    fn merge_hold(&mut self) -> Result<Option<String>> {
        Ok(None)
    }

    /// Whether an interrupt was requested.
    fn interrupt_requested(&mut self) -> bool;
    /// Write the progress line `line`.
    fn progress(&mut self, line: String);
}

/// Take the ready pull request, whose Base branch is `base`, to `goal`
/// through `outside`. Keep it mergeable and its CI green through the Repair
/// loop (see [`RepairLoop::rounds`]), then check it is still open, ready and
/// mergeable, and for [`Goal::Merged`], merge it at the head whose CI was
/// last watched. A merge that fails goes back round the Repair loop and is
/// tried again on the new head; if that round finds nothing to fix, this
/// fails with a [`PolicyRefusal`]. Returns once the pull request has
/// reached `goal`; fails as interrupted if an interrupt was requested before
/// the merge. `repair_cap` is the most Repair sessions this Delivery starts.
pub(super) fn take_to_goal(
    outside: &mut impl Outside,
    base: &str,
    goal: Goal,
    repair_cap: usize,
) -> Result<()> {
    let mut repair_loop = RepairLoop {
        outside,
        base,
        goal,
        budgets: Budgets {
            repair_cap,
            ..Budgets::default()
        },
    };
    let mut watched = repair_loop.rounds()?;
    loop {
        let outside = &mut *repair_loop.outside;
        outside.stage(if goal == Goal::Merged {
            Stage::SelfMerge
        } else {
            Stage::PullRequestReadiness
        });
        outside.ensure_pr_ready_and_mergeable()?;
        if outside.interrupt_requested() {
            bail!("interrupted");
        }
        if goal == Goal::ReadyForReview {
            return Ok(());
        }
        if let Some(reason) = outside.merge_hold()? {
            return Err(MergeHold { reason }.into());
        }
        outside.progress(format!("merging the PR into {base}"));
        outside.stage(Stage::SelfMerge);
        let error = match outside.merge(&watched)? {
            MergeAttempt::Merged => return Ok(()),
            MergeAttempt::Refused(error) => error,
        };
        outside.progress(format!("the merge failed: {error:#}"));
        match repair_loop.round_after_failed_merge(&watched)? {
            Round::NewHead(head) => watched = head,
            Round::NothingToFix => {
                // Only a PR still ready and mergeable is left ready.
                repair_loop.outside.stage(Stage::SelfMerge);
                repair_loop.outside.ensure_pr_ready_and_mergeable()?;
                return Err(error.context(PolicyRefusal::NothingToFix));
            }
        }
    }
}

/// The most Repair sessions a Delivery starts, conflict, CI-fix and review
/// combined. A Custom process may set a lower cap. It cannot set a higher one.
pub(crate) const MAX_REPAIRS: usize = 5;

/// The most times a Delivery goes round again because the Base branch moved while
/// CI ran or since a merge was tried, or, in a Merge run, because Foreign
/// commits arrived, whether or not the merge that follows needs a Repair. A
/// clean merge of the Base branch uses no Repair, so without this a busy Base
/// branch could keep a Delivery going forever. A Base branch that moved while
/// green or absent CI ran is not counted for a Delivery left ready for review
/// whose pull request still merges cleanly, since that Delivery stops there.
const MAX_UPSTREAM_MOVES: usize = 5;

/// What a Delivery has spent of its Repair and upstream-move budgets,
/// across every round of the Repair loop, those after a failed merge
/// included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Budgets {
    repairs: usize,
    upstream_moves: usize,
    /// The most Repairs this Delivery may start.
    repair_cap: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            repairs: 0,
            upstream_moves: 0,
            repair_cap: MAX_REPAIRS,
        }
    }
}

impl Budgets {
    /// Counts the Repair about to start, returning its numbered kind and its
    /// progress line, or fails if it would be one too many.
    fn next_repair(&mut self, cause: &str) -> Result<(Kind, String)> {
        if self.repairs == self.repair_cap {
            bail!("repairs exhausted: {cause}");
        }
        self.repairs += 1;
        let line = format!(
            "{cause}; starting Repair {} of {}",
            self.repairs, self.repair_cap
        );
        Ok((Kind::Repair(self.repairs), line))
    }

    /// Counts a round taken because `upstream` moved, or fails if it would
    /// be one too many.
    fn count_upstream_move(&mut self, upstream: &str) -> Result<()> {
        if self.upstream_moves == MAX_UPSTREAM_MOVES {
            bail!("{upstream} kept moving: merged it again {MAX_UPSTREAM_MOVES} times");
        }
        self.upstream_moves += 1;
        Ok(())
    }

    /// Counts a round taken because `origin/<base>` moved `when`, returning
    /// its progress line.
    fn count_base_move(&mut self, base: &str, when: &str) -> Result<String> {
        let upstream = format!("origin/{base}");
        self.count_upstream_move(&upstream)?;
        Ok(format!("{upstream} moved {when}; merging it again"))
    }

    /// Counts a round taken because the Issue branch on origin, `upstream`,
    /// has new commits, returning its progress line.
    fn count_foreign_commits(&mut self, upstream: &str) -> Result<String> {
        self.count_upstream_move(upstream)?;
        Ok(format!("{upstream} has new commits; merging them in"))
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
struct RepairLoop<'a, O> {
    outside: &'a mut O,
    base: &'a str,
    goal: Goal,
    budgets: Budgets,
}

impl<O: Outside> RepairLoop<'_, O> {
    /// Start the next Repair, `repair`, for `cause`, counting it against the
    /// Repair cap.
    fn repair(&mut self, cause: &str, repair: Repair) -> Result<()> {
        // Exhausted repairs start no session, so record none.
        self.outside.stage(Stage::RepairLoopRound);
        let (kind, line) = self.budgets.next_repair(cause)?;
        self.outside.progress(line);
        self.outside.stage(Stage::RepairSession);
        self.outside.repair(kind, repair)
    }

    /// Whether GitHub reports the pull request mergeable, read in the
    /// readiness stage.
    fn pr_merges_cleanly(&mut self) -> Result<bool> {
        self.outside.stage(Stage::PullRequestReadiness);
        self.outside.pr_mergeable()
    }

    /// Count a round taken because the Base branch moved `when`.
    fn count_base_move(&mut self, when: &str) -> Result<()> {
        let line = self.budgets.count_base_move(self.base, when)?;
        self.outside.progress(line);
        Ok(())
    }

    /// In a Merge run, first take in any Foreign commits. Then merge the
    /// Base branch (never rebase), push, and watch CI on the head commit,
    /// starting a Repair session for a conflict or red
    /// CI and then going round again, since the Base branch may have moved
    /// meanwhile. Green or absent CI also goes round again, in a Merge run, if
    /// the Base branch moved while CI ran or Foreign commits arrived. Left
    /// ready for review, it returns that head without counting the move if
    /// GitHub reports the pull request mergeable, and otherwise goes round
    /// again.
    /// A red check that also fails on the Base branch commit the head last
    /// merged in is an Inherited failure, which no Repair is started for: the
    /// CI-fix Repair is given the branch's own failures, and when there are
    /// none, the loop goes round again if the Base branch moved since, counted
    /// as a Base move, and otherwise once its Base fix has returned, or fails
    /// with the Base fix's cause. A Run that sees no Inherited failures
    /// compares with no Base branch commit.
    /// CI watch owns observation and re-run policy across all rounds. The
    /// loop reports only successfully completed CI-fix Repairs to it.
    /// Returns the head commit whose CI was last watched and found green or
    /// absent. Fails if assessment reports a Declined CI fix, or a Repair
    /// beyond `MAX_REPAIRS`, or a round beyond `MAX_UPSTREAM_MOVES`, would be
    /// needed.
    fn rounds(&mut self) -> Result<String> {
        let base = self.base;
        let mut issue_move_counted = false;
        loop {
            self.outside.stage(Stage::RepairLoopRound);
            if self.goal == Goal::Merged {
                self.take_in_foreign_commits(std::mem::take(&mut issue_move_counted))?;
            }
            let base_commit = match self.outside.merge_base_branch()? {
                Merge::Clean { commit } => commit,
                Merge::Conflicted(pending) => {
                    self.repair("conflict", Repair::Conflict(Upstream::BaseBranch))?;
                    self.outside.ensure_merged(&pending)?;
                    continue;
                }
            };
            self.outside.stage(Stage::Push);
            self.outside.push()?;
            let head = self.outside.head()?;
            let compared_with = self
                .outside
                .sees_inherited_failures()
                .then_some(base_commit.as_str());
            self.outside.stage(Stage::CiWatch);
            let ci = self.outside.assess_ci(&head, compared_with)?;
            match ci {
                Ci::Superseded => {
                    self.budgets.count_upstream_move("Issue branch on origin")?;
                    issue_move_counted = true;
                }
                Ci::Absent | Ci::Passed => {
                    if self.outside.base_branch_moved()? {
                        // Green CI on a head that merges cleanly is enough
                        // for review; only a Self-merge needs the moved Base
                        // branch merged in and watched.
                        if self.goal == Goal::ReadyForReview {
                            if self.pr_merges_cleanly()? {
                                self.outside.progress(format!(
                                    "origin/{base} moved while CI ran; leaving the PR for \
                                     review on {}, which merges cleanly",
                                    ci::short(&head)
                                ));
                                return Ok(head);
                            }
                            self.outside.stage(Stage::RepairLoopRound);
                        }
                        self.count_base_move("while CI ran")?;
                    } else if self.goal == Goal::ReadyForReview
                        || self.outside.new_commits_on_origin()?.commits().is_empty()
                    {
                        return Ok(head);
                    }
                }
                Ci::Failed(failed) => {
                    if !failed.inherited.is_empty() {
                        self.outside.progress(format!(
                            "Inherited failures (also failing on {base} at {}): {}",
                            ci::short(&base_commit),
                            ci::check_names(&failed.inherited)
                        ));
                    }
                    if failed.own.is_empty() {
                        // Someone may have fixed the Base branch since.
                        if self.outside.base_branch_moved()? {
                            self.count_base_move("while CI ran")?;
                            continue;
                        }
                        self.outside.stage(Stage::BaseFix);
                        self.outside.base_fix(&base_commit, &failed)?;
                        continue;
                    }
                    self.repair("CI red", Repair::CiFix(&failed))?;
                    self.outside.ci_fix_completed(head, failed);
                }
            }
        }
    }

    /// Fetch the Issue branch from origin and merge in any Foreign commits on
    /// it (never rebase), counting that as a round and handing a conflict to
    /// a conflict Repair. If the resulting tree differs, a review Repair
    /// reviews it from the head the Run last knew as its own, the local head
    /// before the merge. An identical tree needs no new review. Goes round
    /// again until origin has nothing new, since more may land during either
    /// Repair, and the push after them would be rejected.
    fn take_in_foreign_commits(&mut self, mut first_move_counted: bool) -> Result<()> {
        loop {
            let foreign = self.outside.new_commits_on_origin()?;
            if foreign.commits().is_empty() {
                return Ok(());
            }
            let upstream = foreign.upstream().to_string();
            let own_head = foreign.local_head().to_string();
            let line = if std::mem::take(&mut first_move_counted) {
                format!("{upstream} has new commits; merging them in")
            } else {
                self.budgets.count_foreign_commits(&upstream)?
            };
            self.outside.progress(line);
            for sha in foreign.commits() {
                self.outside
                    .progress(format!("merging new commit {sha} from {upstream}"));
            }
            if let Merge::Conflicted(pending) = self.outside.merge_new_commits(foreign)? {
                self.repair(
                    &format!("conflict with new commits on {upstream}"),
                    Repair::Conflict(Upstream::IssueBranch),
                )?;
                self.outside.ensure_merged(&pending)?;
            }
            if !self.outside.same_tree_as(&own_head)? {
                self.repair(
                    &format!("new commits on {upstream} to review"),
                    Repair::Review { from: &own_head },
                )?;
            }
        }
    }

    /// Go round again after a merge of `watched` failed, counting a Base
    /// branch that moved since as an upstream move. GitHub's error text is never
    /// consulted.
    fn round_after_failed_merge(&mut self, watched: &str) -> Result<Round> {
        self.outside.stage(Stage::RepairLoopRound);
        let before = self.budgets;
        if self.outside.base_branch_moved()? {
            self.count_base_move("since the merge was tried")?;
        }
        let head = self.rounds()?;
        Ok(if head == watched && self.budgets == before {
            Round::NothingToFix
        } else {
            Round::NewHead(head)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use anyhow::anyhow;

    use super::*;
    use crate::github::{Check, Conclusion};

    /// What the Repair loop did outside itself, in the order it did it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Did {
        /// Merged the Base branch in.
        MergeBase,
        /// Merged the new commits on origin in.
        MergeNewCommits(String),
        /// Checked the pending merge of this upstream was finished.
        CheckMerged(&'static str),
        Push,
        /// Watched CI on this head, compared with this Base branch commit.
        Assess(String, Option<String>),
        /// Reported a completed CI-fix Repair of this full head.
        CiFixCompleted(String, Vec<String>, Vec<String>),
        /// Started the Repair session of this kind for this Repair.
        Repair(Kind, Started),
        /// Took the Base fix for these Inherited failures, at this Base
        /// branch commit.
        BaseFix(String, Vec<String>),
        /// Read whether GitHub reports the PR mergeable.
        ReadMergeable,
        /// Checked the PR is open, ready and mergeable.
        CheckPr,
        /// Merged the PR at this head.
        Merge(String),
        /// Wrote this progress line.
        Line(String),
    }

    /// A Repair as the scripted adapter records it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Started {
        Conflict(Upstream),
        /// A CI fix given these failed checks of the branch's own, and these
        /// Inherited failures.
        CiFix(Vec<String>, Vec<String>),
        /// A review from this commit.
        Review(String),
    }

    /// How CI on a head ends, as a script gives it.
    enum Is {
        Absent,
        Passed,
        Superseded,
        /// Red on these checks of the branch's own, and these Inherited
        /// failures.
        Failed(&'static [&'static str], &'static [&'static str]),
    }

    impl Is {
        /// CI as the loop sees it, built fresh.
        fn ci(&self) -> Ci {
            let checks = |names: &[&str]| -> Vec<Check> {
                names
                    .iter()
                    .map(|name| Check {
                        name: name.to_string(),
                        conclusion: Conclusion::Failure,
                        url: None,
                        job: None,
                        workflow_run: None,
                    })
                    .collect()
            };
            match self {
                Is::Absent => Ci::Absent,
                Is::Passed => Ci::Passed,
                Is::Superseded => Ci::Superseded,
                Is::Failed(own, inherited) => Ci::Failed(FailedChecks {
                    own: checks(own),
                    inherited: checks(inherited),
                    on_base: checks(inherited),
                }),
            }
        }
    }

    /// What the outside world answers, call by call. A head or Base branch
    /// commit stays the last one given; a merge is clean, the Base branch
    /// unmoved, the PR mergeable, origin without new commits and a Base fix
    /// returns, unless scripted otherwise.
    #[derive(Default)]
    struct Script {
        heads: VecDeque<&'static str>,
        base_commits: VecDeque<&'static str>,
        /// For each merge of the Base branch, whether it conflicts.
        base_conflicts: VecDeque<bool>,
        /// For each merge of new commits on origin, whether it conflicts.
        new_commit_conflicts: VecDeque<bool>,
        tree_matches: VecDeque<Result<bool, &'static str>>,
        merge_completion_error: Option<&'static str>,
        base_moved: VecDeque<bool>,
        /// For each read of the PR's mergeability, whether GitHub reports it
        /// mergeable.
        mergeable: VecDeque<bool>,
        new_commits: VecDeque<ForeignCommits>,
        assessments: VecDeque<Is>,
        assessment_errors: VecDeque<Option<&'static str>>,
        /// For each Base fix, the cause it fails with, if it does.
        base_fixes: VecDeque<Option<&'static str>>,
        /// For each merge, the error it fails with, if it does.
        merges: VecDeque<Option<&'static str>>,
        merge_gate_error: Option<&'static str>,
        /// The error a merge of the Base branch fails with, if it does.
        base_merge_error: Option<&'static str>,
        /// The error a watch of CI fails with, if it does.
        assessment_error: Option<&'static str>,
        /// The error a Repair session fails with, if it does.
        repair_error: Option<&'static str>,
        sees_no_inherited_failures: bool,
        interrupted: bool,
    }

    /// The outside world as `script` answers it, with what the loop did there
    /// recorded in `did`, and the stages it entered, apart, in `stages`.
    struct Scripted {
        script: Script,
        did: Vec<Did>,
        stages: Vec<Stage>,
    }

    /// The next of `answers`, keeping the last one for every call after it.
    fn sticky(answers: &mut VecDeque<&'static str>, what: &str) -> String {
        let answer = if answers.len() > 1 {
            answers.pop_front()
        } else {
            answers.front().copied()
        };
        answer
            .unwrap_or_else(|| panic!("no {what} scripted"))
            .to_string()
    }

    /// The names of `checks`.
    fn names(checks: &[Check]) -> Vec<String> {
        checks.iter().map(|check| check.name.clone()).collect()
    }

    impl Outside for Scripted {
        /// The upstream whose merge is pending.
        type Pending = &'static str;

        fn stage(&mut self, stage: Stage) {
            self.stages.push(stage);
        }

        fn merge_base_branch(&mut self) -> Result<Merge<&'static str>> {
            self.did.push(Did::MergeBase);
            if let Some(error) = self.script.base_merge_error {
                bail!(error);
            }
            let conflicted = self.script.base_conflicts.pop_front().unwrap_or(false);
            if conflicted {
                return Ok(Merge::Conflicted("origin/main"));
            }
            let commit = if self.script.base_commits.is_empty() {
                "b1".to_string()
            } else {
                sticky(&mut self.script.base_commits, "Base branch commit")
            };
            Ok(Merge::Clean { commit })
        }

        fn merge_new_commits(&mut self, observed: ForeignCommits) -> Result<Merge<&'static str>> {
            self.did
                .push(Did::MergeNewCommits(observed.origin_head().to_string()));
            let conflicted = self
                .script
                .new_commit_conflicts
                .pop_front()
                .unwrap_or(false);
            Ok(if conflicted {
                Merge::Conflicted("origin/issue-7")
            } else {
                Merge::Clean {
                    commit: observed.origin_head().to_string(),
                }
            })
        }

        fn ensure_merged(&mut self, pending: &&'static str) -> Result<()> {
            self.did.push(Did::CheckMerged(pending));
            match self.script.merge_completion_error {
                Some(error) => bail!(error),
                None => Ok(()),
            }
        }

        fn push(&mut self) -> Result<()> {
            self.did.push(Did::Push);
            Ok(())
        }

        fn head(&mut self) -> Result<String> {
            Ok(sticky(&mut self.script.heads, "head"))
        }

        fn same_tree_as(&mut self, _: &str) -> Result<bool> {
            self.script
                .tree_matches
                .pop_front()
                .unwrap_or(Ok(false))
                .map_err(|error| anyhow!(error))
        }

        fn base_branch_moved(&mut self) -> Result<bool> {
            Ok(self.script.base_moved.pop_front().unwrap_or(false))
        }

        fn new_commits_on_origin(&mut self) -> Result<ForeignCommits> {
            Ok(self
                .script
                .new_commits
                .pop_front()
                .unwrap_or_else(|| ForeignCommits::fixture("h1", &[])))
        }

        fn assess_ci(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci> {
            assert_eq!(self.stages.last(), Some(&Stage::CiWatch));
            self.did
                .push(Did::Assess(head.to_string(), base_commit.map(String::from)));
            if let Some(error) = self
                .script
                .assessment_errors
                .pop_front()
                .flatten()
                .or(self.script.assessment_error)
            {
                bail!(error);
            }
            Ok(self
                .script
                .assessments
                .pop_front()
                .expect("no assessment scripted")
                .ci())
        }

        fn ci_fix_completed(&mut self, head: String, failed: FailedChecks) {
            self.did.push(Did::CiFixCompleted(
                head,
                names(&failed.own),
                names(&failed.inherited),
            ));
        }

        fn repair(&mut self, kind: Kind, repair: Repair) -> Result<()> {
            let started = match repair {
                Repair::Conflict(upstream) => Started::Conflict(upstream),
                Repair::CiFix(failed) => {
                    Started::CiFix(names(&failed.own), names(&failed.inherited))
                }
                Repair::Review { from } => Started::Review(from.to_string()),
            };
            self.did.push(Did::Repair(kind, started));
            match self.script.repair_error {
                Some(error) => bail!(error),
                None => Ok(()),
            }
        }

        fn sees_inherited_failures(&mut self) -> bool {
            !self.script.sees_no_inherited_failures
        }

        fn base_fix(&mut self, base_commit: &str, failed: &FailedChecks) -> Result<()> {
            self.did.push(Did::BaseFix(
                base_commit.to_string(),
                names(&failed.inherited),
            ));
            match self.script.base_fixes.pop_front().flatten() {
                Some(cause) => Err(anyhow!(cause)),
                None => Ok(()),
            }
        }

        fn pr_mergeable(&mut self) -> Result<bool> {
            self.did.push(Did::ReadMergeable);
            Ok(self.script.mergeable.pop_front().unwrap_or(true))
        }

        fn ensure_pr_ready_and_mergeable(&mut self) -> Result<()> {
            self.did.push(Did::CheckPr);
            Ok(())
        }

        fn merge(&mut self, head: &str) -> Result<MergeAttempt> {
            if let Some(error) = self.script.merge_gate_error {
                bail!(error);
            }
            self.did.push(Did::Merge(head.to_string()));
            match self.script.merges.pop_front().expect("no merge scripted") {
                Some(error) => Ok(MergeAttempt::Refused(anyhow!(error))),
                None => Ok(MergeAttempt::Merged),
            }
        }

        fn interrupt_requested(&mut self) -> bool {
            self.script.interrupted
        }

        fn progress(&mut self, line: String) {
            self.did.push(Did::Line(line));
        }
    }

    /// Take a PR into `main` to `goal` against `script`: what it came to,
    /// and what the loop did.
    fn deliver(goal: Goal, script: Script) -> (Result<()>, Vec<Did>) {
        let (delivered, did, _) = deliver_with_stages(goal, script);
        (delivered, did)
    }

    /// [`deliver`], with the stages the loop entered, in order.
    fn deliver_with_stages(goal: Goal, script: Script) -> (Result<()>, Vec<Did>, Vec<Stage>) {
        let mut outside = Scripted {
            script,
            did: Vec::new(),
            stages: Vec::new(),
        };
        let delivered = take_to_goal(&mut outside, "main", goal, MAX_REPAIRS);
        (delivered, outside.did, outside.stages)
    }

    /// The stage in force when a Delivery to `goal` against `script` failed,
    /// and its cause.
    fn failed_in(goal: Goal, script: Script) -> (Stage, String) {
        let (delivered, _, stages) = deliver_with_stages(goal, script);
        let stage = *stages.last().expect("no stage entered");
        (stage, cause(delivered))
    }

    /// `answers` as a script's queue.
    fn script<T>(answers: impl IntoIterator<Item = T>) -> VecDeque<T> {
        answers.into_iter().collect()
    }

    /// The progress line `line`.
    fn line(line: &str) -> Did {
        Did::Line(line.to_string())
    }

    /// A watch of CI on `head`, compared with the Base branch commit `b1`.
    fn assessment(head: &str) -> Did {
        Did::Assess(head.to_string(), Some("b1".to_string()))
    }

    /// The numbered Repair session `n` for `started`.
    fn repair(n: usize, started: Started) -> Did {
        Did::Repair(Kind::Repair(n), started)
    }

    /// A CI-fix Repair given the branch's own failures `own`, and the
    /// Inherited failures `inherited`.
    fn ci_fix(own: &[&str], inherited: &[&str]) -> Started {
        let names = |checks: &[&str]| checks.iter().map(|name| name.to_string()).collect();
        Started::CiFix(names(own), names(inherited))
    }

    /// A round from the merge of the Base branch to the watch of `head`.
    fn round(head: &str) -> [Did; 3] {
        [Did::MergeBase, Did::Push, assessment(head)]
    }

    /// The Self-merge of `head`, after the PR check.
    fn merge(head: &str) -> [Did; 3] {
        [
            Did::CheckPr,
            line("merging the PR into main"),
            Did::Merge(head.to_string()),
        ]
    }

    /// The progress lines among `did`.
    fn lines(did: &[Did]) -> Vec<&str> {
        did.iter()
            .filter_map(
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "keeps only progress lines; a new kind of step must be listed to count"
                )]
                |did| match did {
                    Did::Line(line) => Some(line.as_str()),
                    _ => None,
                },
            )
            .collect()
    }

    /// The completed CI-fix Repair notification with its original facts.
    fn completed(head: &str, own: &[&str], inherited: &[&str]) -> Did {
        Did::CiFixCompleted(
            head.to_string(),
            own.iter().map(|name| name.to_string()).collect(),
            inherited.iter().map(|name| name.to_string()).collect(),
        )
    }

    /// The cause `delivered` failed with, every context included.
    fn cause(delivered: Result<()>) -> String {
        format!("{:#}", delivered.unwrap_err())
    }

    #[test]
    fn green_or_absent_ci_with_nothing_moved_reaches_the_goal() {
        for is in [Is::Passed, Is::Absent] {
            let (delivered, did) = deliver(
                Goal::Merged,
                Script {
                    heads: script(["h1"]),
                    assessments: script([is]),
                    merges: script([None]),
                    ..Script::default()
                },
            );

            delivered.unwrap();
            assert_eq!(did, [round("h1").as_slice(), &merge("h1")].concat());
        }
    }

    #[test]
    fn a_run_left_ready_for_review_never_merges_or_takes_in_foreign_commits() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                new_commits: script([
                    ForeignCommits::fixture("h1", &["f1"]),
                    ForeignCommits::fixture("h1", &["f1"]),
                ]),
                assessments: script([Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(did, [round("h1").as_slice(), &[Did::CheckPr]].concat());
    }

    #[test]
    fn a_conflict_with_the_base_branch_is_handed_to_a_conflict_repair_then_round_again() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                base_conflicts: script([true]),
                assessments: script([Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                &[
                    Did::MergeBase,
                    line("conflict; starting Repair 1 of 5"),
                    repair(1, Started::Conflict(Upstream::BaseBranch)),
                    Did::CheckMerged("origin/main"),
                ][..],
                &round("h1"),
                &[Did::CheckPr],
            ]
            .concat()
        );
    }

    #[test]
    fn red_ci_is_handed_to_a_ci_fix_repair_with_only_its_own_failures_and_the_new_head_watched() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1", "h2"]),
                assessments: script([Is::Failed(&["test"], &["lint"]), Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                &round("h1")[..],
                &[
                    line("Inherited failures (also failing on main at b1): lint"),
                    line("CI red; starting Repair 1 of 5"),
                    repair(1, ci_fix(&["test"], &["lint"])),
                    completed("h1", &["test"], &["lint"]),
                ],
                &round("h2"),
                &[Did::CheckPr],
            ]
            .concat()
        );
    }

    #[test]
    fn assessment_after_a_ci_fix_waits_for_a_successful_base_merge_despite_conflict_repairs() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["full-head-before-repair", "full-head-after-conflict"]),
                base_commits: script(["b1", "selected-after-conflict"]),
                base_conflicts: script([false, true, false]),
                assessments: script([Is::Failed(&["test"], &[]), Is::Passed]),
                ..Script::default()
            },
        );
        delivered.unwrap();
        assert_eq!(
            did,
            [
                round("full-head-before-repair").to_vec(),
                vec![
                    line("CI red; starting Repair 1 of 5"),
                    repair(1, ci_fix(&["test"], &[])),
                    completed("full-head-before-repair", &["test"], &[]),
                    Did::MergeBase,
                    line("conflict; starting Repair 2 of 5"),
                    repair(2, Started::Conflict(Upstream::BaseBranch)),
                    Did::CheckMerged("origin/main"),
                    Did::MergeBase,
                    Did::Push,
                    Did::Assess(
                        "full-head-after-conflict".to_string(),
                        Some("selected-after-conflict".to_string())
                    ),
                    Did::CheckPr,
                ]
            ]
            .concat()
        );
    }

    #[test]
    fn a_failed_ci_fix_never_notifies_ci_watch() {
        let (delivered, did, stages) = deliver_with_stages(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Failed(&["test"], &[])]),
                repair_error: Some("CI-fix session or PR postcheck failed"),
                ..Script::default()
            },
        );
        assert_eq!(cause(delivered), "CI-fix session or PR postcheck failed");
        assert_eq!(
            did,
            [
                round("h1").to_vec(),
                vec![
                    line("CI red; starting Repair 1 of 5"),
                    repair(1, ci_fix(&["test"], &[]))
                ]
            ]
            .concat()
        );
        assert_eq!(stages.last(), Some(&Stage::RepairSession));
    }

    #[test]
    fn a_declined_ci_fix_from_assessment_stops_in_ci_watch_without_another_repair() {
        let (delivered, did, stages) = deliver_with_stages(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Failed(&["test"], &[])]),
                assessment_errors: script([
                    None,
                    Some("CI red on h1 and the Repair found nothing to fix on the branch"),
                ]),
                ..Script::default()
            },
        );
        assert_eq!(
            cause(delivered),
            "CI red on h1 and the Repair found nothing to fix on the branch"
        );
        assert_eq!(
            did,
            [
                round("h1").to_vec(),
                vec![
                    line("CI red; starting Repair 1 of 5"),
                    repair(1, ci_fix(&["test"], &[])),
                    completed("h1", &["test"], &[])
                ],
                round("h1").to_vec()
            ]
            .concat()
        );
        assert_eq!(stages.last(), Some(&Stage::CiWatch));
    }

    #[test]
    fn each_assessment_uses_the_commit_returned_by_its_base_merge() {
        for next_base in ["sampled-base", "rewound-base"] {
            let (delivered, did) = deliver(
                Goal::ReadyForReview,
                Script {
                    heads: script(["h1"]),
                    base_commits: script(["sampled-base", next_base]),
                    assessments: script([Is::Failed(&["test"], &[]), Is::Passed]),
                    ..Script::default()
                },
            );
            delivered.unwrap();
            let assessments: Vec<_> = did
                .iter()
                .filter(|did| matches!(did, Did::Assess(..)))
                .cloned()
                .collect();
            assert_eq!(
                assessments,
                [
                    Did::Assess("h1".to_string(), Some("sampled-base".to_string())),
                    Did::Assess("h1".to_string(), Some(next_base.to_string()))
                ]
            );
        }
    }

    #[test]
    fn a_refused_self_merge_reassesses_ci_before_another_repair() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h1", "h2"]),
                assessments: script([Is::Passed, Is::Failed(&["test"], &[]), Is::Passed]),
                merges: script([Some("merge refused"), None]),
                ..Script::default()
            },
        );
        delivered.unwrap();
        assert_eq!(
            did,
            [
                round("h1").to_vec(),
                merge("h1").to_vec(),
                vec![line("the merge failed: merge refused")],
                round("h1").to_vec(),
                vec![
                    line("CI red; starting Repair 1 of 5"),
                    repair(1, ci_fix(&["test"], &[])),
                    completed("h1", &["test"], &[])
                ],
                round("h2").to_vec(),
                merge("h2").to_vec()
            ]
            .concat()
        );
    }

    /// What the loop does up to and including the Inherited failures line
    /// for `test` on `h1`.
    fn up_to_inherited_failures() -> Vec<Did> {
        [
            &round("h1")[..],
            &[line(
                "Inherited failures (also failing on main at b1): test",
            )],
        ]
        .concat()
    }

    #[test]
    fn inherited_failures_only_go_round_again_as_a_base_move_if_the_base_branch_moved() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1", "h2"]),
                base_commits: script(["b1", "b2"]),
                base_moved: script([true]),
                assessments: script([Is::Failed(&[], &["test"]), Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                up_to_inherited_failures(),
                vec![
                    line("origin/main moved while CI ran; merging it again"),
                    Did::MergeBase,
                    Did::Push,
                    Did::Assess("h2".to_string(), Some("b2".to_string())),
                    Did::CheckPr,
                ],
            ]
            .concat()
        );
    }

    #[test]
    fn inherited_failures_only_take_the_base_fix_then_go_round_again() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Failed(&[], &["test"]), Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                up_to_inherited_failures(),
                vec![Did::BaseFix("b1".to_string(), vec!["test".to_string()])],
                round("h1").into(),
                vec![Did::CheckPr],
            ]
            .concat()
        );
    }

    #[test]
    fn a_failed_base_fix_is_the_runs_cause() {
        let fails = "CI red on test, which also fails on main at b1; fix main first";
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Failed(&[], &["test"])]),
                base_fixes: script([Some(fails)]),
                ..Script::default()
            },
        );

        assert_eq!(cause(delivered), fails);
        assert_eq!(
            did,
            [
                up_to_inherited_failures(),
                vec![Did::BaseFix("b1".to_string(), vec!["test".to_string()])],
            ]
            .concat()
        );
    }

    #[test]
    fn a_run_that_sees_no_inherited_failures_assesses_ci_with_no_base_branch_commit() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Passed]),
                sees_no_inherited_failures: true,
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                Did::MergeBase,
                Did::Push,
                Did::Assess("h1".to_string(), None),
                Did::CheckPr
            ]
        );
    }

    #[test]
    fn in_a_merge_run_the_base_branch_moving_while_ci_ran_goes_round_again_as_a_base_move() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2"]),
                base_moved: script([true]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                &round("h1")[..],
                &[line("origin/main moved while CI ran; merging it again")],
                &round("h2"),
                &merge("h2"),
            ]
            .concat()
        );
    }

    #[test]
    fn a_run_left_ready_for_review_ends_on_green_ci_when_the_moved_base_branch_merges_cleanly() {
        for is in [Is::Passed, Is::Absent] {
            let (delivered, did) = deliver(
                Goal::ReadyForReview,
                Script {
                    heads: script(["h1234567890", "h2"]),
                    base_moved: script([true]),
                    assessments: script([is]),
                    ..Script::default()
                },
            );

            delivered.unwrap();
            assert_eq!(
                did,
                [
                    &[Did::MergeBase, Did::Push, assessment("h1234567890")][..],
                    &[
                        Did::ReadMergeable,
                        line(
                            "origin/main moved while CI ran; leaving the PR for review on \
                             h123456, which merges cleanly"
                        ),
                        Did::CheckPr,
                    ],
                ]
                .concat()
            );
        }
    }

    #[test]
    fn a_run_left_ready_for_review_goes_round_again_as_a_base_move_if_the_pr_does_not_merge() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1", "h2"]),
                base_moved: script([true]),
                base_conflicts: script([false, true]),
                mergeable: script([false]),
                assessments: script([Is::Passed, Is::Passed]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                &round("h1")[..],
                &[
                    Did::ReadMergeable,
                    line("origin/main moved while CI ran; merging it again"),
                    Did::MergeBase,
                    line("conflict; starting Repair 1 of 5"),
                    repair(1, Started::Conflict(Upstream::BaseBranch)),
                    Did::CheckMerged("origin/main"),
                ],
                &round("h2"),
                &[Did::CheckPr],
            ]
            .concat()
        );
    }

    /// What the loop does to take in the Foreign commit `sha`, merged
    /// cleanly on top of `own_head`, as Repair `n`.
    fn foreign_commit(sha: &str, own_head: &str, n: usize) -> Vec<Did> {
        vec![
            line("origin/issue-7 has new commits; merging them in"),
            line(&format!("merging new commit {sha} from origin/issue-7")),
            Did::MergeNewCommits(sha.to_string()),
            line(&format!(
                "new commits on origin/issue-7 to review; starting Repair {n} of 5"
            )),
            repair(n, Started::Review(own_head.to_string())),
        ]
    }

    #[test]
    fn tree_identical_foreign_batches_leave_repair_slots_for_changed_trees() {
        let (delivered, did) =
            deliver(
                Goal::Merged,
                Script {
                    heads: script(["h5"]),
                    new_commits: script((1..=5).map(|n| {
                        ForeignCommits::fixture(&format!("h{}", n - 1), &[&format!("f{n}")])
                    })),
                    tree_matches: script([Ok(true), Ok(true), Ok(true), Ok(true), Ok(false)]),
                    assessments: script([Is::Passed]),
                    merges: script([None]),
                    ..Script::default()
                },
            );

        delivered.unwrap();
        let repairs: Vec<_> = did
            .iter()
            .filter(|did| matches!(did, Did::Repair(..)))
            .collect();
        assert_eq!(repairs, [&repair(1, Started::Review("h4".to_string()))]);
        assert_eq!(
            did.iter()
                .filter(|did| matches!(did, Did::MergeNewCommits(..)))
                .count(),
            5
        );
    }

    #[test]
    fn tree_identical_foreign_batches_still_spend_the_upstream_move_budget() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                new_commits: script(
                    (1..=6).map(|n| ForeignCommits::fixture(&format!("h{n}"), &[&format!("f{n}")])),
                ),
                tree_matches: script([Ok(true); 5]),
                ..Script::default()
            },
        );

        assert_eq!(
            cause(delivered),
            "origin/issue-7 kept moving: merged it again 5 times"
        );
        assert_eq!(
            did.iter()
                .filter(|did| matches!(did, Did::MergeNewCommits(..)))
                .count(),
            5
        );
        assert!(!did.iter().any(|did| matches!(did, Did::Repair(..))));
    }

    #[test]
    fn a_failed_tree_comparison_cannot_authorize_self_merge() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                new_commits: script([ForeignCommits::fixture("h1", &["f1"])]),
                tree_matches: script([Err("cannot compare trees")]),
                ..Script::default()
            },
        );

        assert_eq!(cause(delivered), "cannot compare trees");
        assert!(
            !did.iter()
                .any(|did| matches!(did, Did::Merge(..) | Did::Assess(..) | Did::Repair(..)))
        );
    }

    #[test]
    fn an_identical_tree_does_not_excuse_an_unfinished_foreign_merge() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                new_commits: script([ForeignCommits::fixture("h1", &["f1"])]),
                new_commit_conflicts: script([true]),
                merge_completion_error: Some("merge is still in progress"),
                tree_matches: script([Ok(true)]),
                ..Script::default()
            },
        );

        assert_eq!(cause(delivered), "merge is still in progress");
        assert_eq!(did.last(), Some(&Did::CheckMerged("origin/issue-7")));
        assert!(did.contains(&repair(1, Started::Conflict(Upstream::IssueBranch))));
    }

    #[test]
    fn a_merge_run_merges_foreign_commits_in_and_reviews_them_while_origin_has_more() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h3"]),
                new_commits: script([
                    ForeignCommits::fixture("h1", &["f1", "f2"]),
                    ForeignCommits::fixture("h2", &["f3"]),
                ]),
                assessments: script([Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                vec![
                    line("origin/issue-7 has new commits; merging them in"),
                    line("merging new commit f1 from origin/issue-7"),
                    line("merging new commit f2 from origin/issue-7"),
                    Did::MergeNewCommits("f2".to_string()),
                    line("new commits on origin/issue-7 to review; starting Repair 1 of 5"),
                    repair(1, Started::Review("h1".to_string())),
                ],
                foreign_commit("f3", "h2", 2),
                round("h3").into(),
                merge("h3").into(),
            ]
            .concat()
        );
    }

    #[test]
    fn tree_identical_foreign_commits_reconcile_without_a_review_repair() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["f1"]),
                new_commits: script([ForeignCommits::fixture("own", &["f1"])]),
                tree_matches: script([Ok(true)]),
                assessments: script([Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert!(!did.iter().any(|did| matches!(did, Did::Repair(..))));
        assert!(did.contains(&Did::MergeNewCommits("f1".to_string())));
        assert!(did.contains(&Did::Push));
        assert!(did.contains(&Did::Merge("f1".to_string())));
    }

    #[test]
    fn an_empty_foreign_batch_between_changed_batches_spends_no_repair_slot() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["f3"]),
                new_commits: script([
                    ForeignCommits::fixture("own", &["f1"]),
                    ForeignCommits::fixture("reviewed", &["f2"]),
                    ForeignCommits::fixture("f2", &["f3"]),
                ]),
                tree_matches: script([Ok(false), Ok(true), Ok(false)]),
                assessments: script([Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        let repairs: Vec<_> = did
            .iter()
            .filter(|did| matches!(did, Did::Repair(..)))
            .collect();
        assert_eq!(
            repairs,
            [
                &repair(1, Started::Review("own".to_string())),
                &repair(2, Started::Review("f2".to_string())),
            ]
        );
        assert!(did.contains(&Did::MergeNewCommits("f2".to_string())));
        assert!(did.contains(&Did::Merge("f3".to_string())));
    }

    #[test]
    fn an_identical_tree_after_conflict_repair_needs_no_review_repair() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["resolved"]),
                new_commits: script([ForeignCommits::fixture("own", &["f1"])]),
                new_commit_conflicts: script([true]),
                tree_matches: script([Ok(true)]),
                assessments: script([Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did.iter()
                .filter(|did| matches!(did, Did::Repair(..)))
                .collect::<Vec<_>>(),
            [&repair(1, Started::Conflict(Upstream::IssueBranch))]
        );
        assert!(did.contains(&Did::CheckMerged("origin/issue-7")));
        assert!(did.contains(&Did::Merge("resolved".to_string())));
    }

    #[test]
    fn an_identical_foreign_tree_can_reconcile_after_all_repairs_are_spent() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2", "h3", "h4", "h5", "h6"]),
                assessments: script(
                    (0..5)
                        .map(|_| Is::Failed(&["test"], &[]))
                        .chain([Is::Passed]),
                ),
                new_commits: script(
                    (0..5)
                        .map(|_| ForeignCommits::fixture("own", &[]))
                        .chain([ForeignCommits::fixture("h6", &["f1"])]),
                ),
                tree_matches: script([Ok(true)]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did.iter()
                .filter(|did| matches!(did, Did::Repair(..)))
                .count(),
            5
        );
        assert!(
            !did.iter()
                .any(|did| matches!(did, Did::Repair(_, Started::Review(_))))
        );
        assert!(did.contains(&Did::MergeNewCommits("f1".to_string())));
    }

    #[test]
    fn foreign_commits_that_arrive_while_ci_ran_go_round_again() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2"]),
                new_commits: script([
                    ForeignCommits::fixture("h1", &[]),
                    ForeignCommits::fixture("h1", &["f1"]),
                    ForeignCommits::fixture("h1", &["f1"]),
                ]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                round("h1").into(),
                foreign_commit("f1", "h1", 1),
                round("h2").into(),
                merge("h2").into(),
            ]
            .concat()
        );
    }

    #[test]
    fn a_conflict_with_foreign_commits_is_handed_to_a_conflict_repair_then_reviewed() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h2"]),
                new_commits: script([ForeignCommits::fixture("h1", &["f1"])]),
                new_commit_conflicts: script([true]),
                assessments: script([Is::Passed]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                vec![
                    line("origin/issue-7 has new commits; merging them in"),
                    line("merging new commit f1 from origin/issue-7"),
                    Did::MergeNewCommits("f1".to_string()),
                    line("conflict with new commits on origin/issue-7; starting Repair 1 of 5"),
                    repair(1, Started::Conflict(Upstream::IssueBranch)),
                    Did::CheckMerged("origin/issue-7"),
                    line("new commits on origin/issue-7 to review; starting Repair 2 of 5"),
                    repair(2, Started::Review("h1".to_string())),
                ],
                round("h2").into(),
                merge("h2").into(),
            ]
            .concat()
        );
    }

    #[test]
    fn every_kind_of_repair_shares_one_cap_and_the_one_too_many_fails_with_its_cause() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2", "h3", "h4"]),
                new_commits: script([ForeignCommits::fixture("h0", &["f1"])]),
                base_conflicts: script([true]),
                assessments: script([
                    Is::Failed(&["test"], &[]),
                    Is::Failed(&["test"], &[]),
                    Is::Failed(&["test"], &[]),
                    Is::Failed(&["test"], &[]),
                ]),
                ..Script::default()
            },
        );

        assert_eq!(cause(delivered), "repairs exhausted: CI red");
        assert_eq!(
            lines(&did),
            [
                "origin/issue-7 has new commits; merging them in",
                "merging new commit f1 from origin/issue-7",
                "new commits on origin/issue-7 to review; starting Repair 1 of 5",
                "conflict; starting Repair 2 of 5",
                "CI red; starting Repair 3 of 5",
                "CI red; starting Repair 4 of 5",
                "CI red; starting Repair 5 of 5",
            ]
        );
        assert_eq!(did.last(), Some(&assessment("h4")));
    }

    #[test]
    fn base_moves_and_foreign_commits_share_one_upstream_move_budget() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                new_commits: script([ForeignCommits::fixture("h0", &["f1"])]),
                base_moved: script([true; 5]),
                assessments: script((0..5).map(|_| Is::Passed)),
                ..Script::default()
            },
        );

        assert_eq!(
            cause(delivered),
            "origin/main kept moving: merged it again 5 times"
        );
        assert_eq!(
            lines(&did),
            [
                "origin/issue-7 has new commits; merging them in",
                "merging new commit f1 from origin/issue-7",
                "new commits on origin/issue-7 to review; starting Repair 1 of 5",
                "origin/main moved while CI ran; merging it again",
                "origin/main moved while CI ran; merging it again",
                "origin/main moved while CI ran; merging it again",
                "origin/main moved while CI ran; merging it again",
            ]
        );
    }

    #[test]
    fn a_sixth_foreign_batch_is_refused_before_its_merge() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                new_commits: script(
                    (1..=6).map(|n| ForeignCommits::fixture(&format!("h{n}"), &[&format!("f{n}")])),
                ),
                ..Script::default()
            },
        );

        assert_eq!(
            cause(delivered),
            "origin/issue-7 kept moving: merged it again 5 times"
        );
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "the test asserts on merges only; the loop's other steps are not under test"
        )]
        let merged: Vec<_> = did
            .iter()
            .filter_map(|did| match did {
                Did::MergeNewCommits(head) => Some(head.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(merged, ["f1", "f2", "f3", "f4", "f5"]);
    }

    #[test]
    fn review_budget_refusal_still_follows_the_allowed_foreign_merge() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2", "h3", "h4", "h5"]),
                assessments: script((0..5).map(|_| Is::Failed(&["test"], &[]))),
                new_commits: script(
                    (0..5)
                        .map(|_| ForeignCommits::fixture("own", &[]))
                        .chain([ForeignCommits::fixture("h6", &["f1"])]),
                ),
                ..Script::default()
            },
        );

        assert_eq!(
            cause(delivered),
            "repairs exhausted: new commits on origin/issue-7 to review"
        );
        assert_eq!(did.last(), Some(&Did::MergeNewCommits("f1".to_string())));
        assert!(
            !did.iter()
                .any(|did| matches!(did, Did::Repair(_, Started::Review(_))))
        );
    }

    #[test]
    fn a_terminal_pre_merge_gate_error_never_enters_another_round_or_policy_refusal() {
        let (ended, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Passed]),
                merge_gate_error: Some("PR targets develop, not main"),
                ..Script::default()
            },
        );
        let error = ended.unwrap_err();
        assert_eq!(error.to_string(), "PR targets develop, not main");
        assert!(!error.is::<PolicyRefusal>());
        assert_eq!(
            did.iter()
                .filter(|call| matches!(call, Did::Assess(..)))
                .count(),
            1
        );
        assert!(
            !did.iter()
                .any(|call| matches!(call, Did::Merge(..) | Did::Repair(..)))
        );
    }

    #[test]
    fn a_failed_merge_goes_round_again_counting_a_base_move_and_retries_on_the_new_head() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2"]),
                base_moved: script([false, true, false]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([Some("Base branch was modified"), None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                &round("h1")[..],
                &merge("h1"),
                &[
                    line("the merge failed: Base branch was modified"),
                    line("origin/main moved since the merge was tried; merging it again"),
                ],
                &round("h2"),
                &merge("h2"),
            ]
            .concat()
        );
    }

    #[test]
    fn failed_merges_that_keep_moving_the_base_branch_spend_its_budget_and_are_no_policy_refusal() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2"]),
                // Still after CI, then moved since the merge was tried.
                base_moved: script([false, true].repeat(6)),
                assessments: script((0..6).map(|_| Is::Passed)),
                merges: script((0..6).map(|_| Some("Base branch was modified"))),
                ..Script::default()
            },
        );

        let error = delivered.unwrap_err();
        assert!(!error.is::<PolicyRefusal>(), "{error:#}");
        assert_eq!(
            format!("{error:#}"),
            "origin/main kept moving: merged it again 5 times"
        );
        assert_eq!(
            did.iter()
                .filter(|did| matches!(did, Did::Merge(_)))
                .count(),
            6
        );
        assert_eq!(
            lines(&did)
                .iter()
                .filter(|line| line.starts_with("origin/main moved since the merge"))
                .count(),
            5
        );
    }

    #[test]
    fn a_failed_merge_whose_round_finds_nothing_to_fix_is_a_policy_refusal() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([Some("refused by a ruleset")]),
                ..Script::default()
            },
        );

        let error = delivered.unwrap_err();
        assert!(error.is::<PolicyRefusal>(), "{error:#}");
        assert_eq!(
            did,
            [
                &round("h1")[..],
                &merge("h1"),
                &[line("the merge failed: refused by a ruleset")],
                &round("h1"),
                &[Did::CheckPr],
            ]
            .concat()
        );
    }

    #[test]
    fn a_failed_merge_whose_round_needed_a_repair_retries_on_the_same_head() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                base_conflicts: script([false, true]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([Some("not mergeable"), None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            lines(&did),
            [
                "merging the PR into main",
                "the merge failed: not mergeable",
                "conflict; starting Repair 1 of 5",
                "merging the PR into main",
            ]
        );
        assert_eq!(did.last(), Some(&Did::Merge("h1".to_string())));
    }

    #[test]
    fn an_interrupt_before_the_merge_fails_as_interrupted() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Passed]),
                interrupted: true,
                ..Script::default()
            },
        );

        assert_eq!(cause(delivered), "interrupted");
        assert_eq!(did, [round("h1").as_slice(), &[Did::CheckPr]].concat());
    }

    #[test]
    fn repairs_of_every_kind_share_one_cap_and_the_one_too_many_names_its_cause() {
        let mut budgets = Budgets::default();
        for n in 1..=MAX_REPAIRS {
            let cause = if n % 2 == 0 { "conflict" } else { "CI red" };
            assert_eq!(
                budgets.next_repair(cause).unwrap(),
                (
                    Kind::Repair(n),
                    format!("{cause}; starting Repair {n} of {MAX_REPAIRS}")
                )
            );
        }

        for cause in ["new commits on origin/issue-7 to review", "conflict"] {
            let error = budgets.next_repair(cause).unwrap_err();
            assert_eq!(error.to_string(), format!("repairs exhausted: {cause}"));
        }
        assert_eq!(budgets.repairs, MAX_REPAIRS);
    }

    #[test]
    fn superseded_heads_count_once_per_upstream_move_without_ci_fix_repairs() {
        for moves in [MAX_UPSTREAM_MOVES, MAX_UPSTREAM_MOVES + 1] {
            let mut assessments: VecDeque<_> = (0..moves).map(|_| Is::Superseded).collect();
            assessments.push_back(Is::Passed);
            let mut new_commits = VecDeque::from([ForeignCommits::fixture("h1", &[])]);
            for _ in 0..moves {
                new_commits.push_back(ForeignCommits::fixture("h1", &["h2"]));
                new_commits.push_back(ForeignCommits::fixture("h2", &[]));
            }
            let (delivered, did) = deliver(
                Goal::Merged,
                Script {
                    heads: script(["h1"]),
                    assessments,
                    new_commits,
                    tree_matches: (0..moves).map(|_| Ok(true)).collect(),
                    merges: script([None]),
                    ..Script::default()
                },
            );
            if moves == MAX_UPSTREAM_MOVES {
                assert!(delivered.is_ok(), "{delivered:?}");
                assert!(did.contains(&Did::Merge("h1".into())));
            } else {
                assert_eq!(
                    cause(delivered),
                    "Issue branch on origin kept moving: merged it again 5 times"
                );
                assert!(!did.iter().any(|did| matches!(did, Did::Merge(_))));
            }
            assert!(
                !did.iter()
                    .any(|did| matches!(did, Did::Repair(..) | Did::CiFixCompleted(..)))
            );
        }
    }

    #[test]
    fn upstream_moves_of_the_base_or_the_issue_branch_share_one_budget() {
        let mut budgets = Budgets::default();
        for _ in 1..MAX_UPSTREAM_MOVES {
            assert_eq!(
                budgets.count_base_move("main", "while CI ran").unwrap(),
                "origin/main moved while CI ran; merging it again"
            );
        }
        assert_eq!(
            budgets.count_foreign_commits("origin/issue-7").unwrap(),
            "origin/issue-7 has new commits; merging them in"
        );

        let error = budgets.count_base_move("main", "while CI ran").unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/main kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
        let error = budgets.count_foreign_commits("origin/issue-7").unwrap_err();
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
                upstream_moves: MAX_UPSTREAM_MOVES,
                repair_cap: MAX_REPAIRS,
            }
        );
    }

    #[test]
    fn a_failed_merge_of_the_base_branch_stops_in_a_repair_loop_round() {
        assert_eq!(
            failed_in(
                Goal::ReadyForReview,
                Script {
                    base_merge_error: Some("could not fetch origin/main"),
                    ..Script::default()
                },
            ),
            (
                Stage::RepairLoopRound,
                "could not fetch origin/main".to_string()
            )
        );
    }

    #[test]
    fn a_run_left_ready_for_review_reads_mergeability_in_readiness_and_counts_an_unmergeable_pr_in_a_round()
     {
        let (delivered, _, stages) = deliver_with_stages(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                base_moved: script([true; MAX_UPSTREAM_MOVES + 1]),
                mergeable: script([false; MAX_UPSTREAM_MOVES + 1]),
                assessments: script((0..=MAX_UPSTREAM_MOVES).map(|_| Is::Passed)),
                ..Script::default()
            },
        );

        assert_eq!(
            cause(delivered),
            "origin/main kept moving: merged it again 5 times"
        );
        assert_eq!(
            stages[stages.len() - 3..],
            [
                Stage::CiWatch,
                Stage::PullRequestReadiness,
                Stage::RepairLoopRound
            ]
        );
    }

    #[test]
    fn a_failed_ci_watch_stops_in_the_ci_watch() {
        assert_eq!(
            failed_in(
                Goal::ReadyForReview,
                Script {
                    heads: script(["h1"]),
                    assessment_error: Some("gh run list failed"),
                    ..Script::default()
                },
            ),
            (Stage::CiWatch, "gh run list failed".to_string())
        );
    }

    #[test]
    fn a_failed_repair_session_stops_in_the_repair_session() {
        assert_eq!(
            failed_in(
                Goal::ReadyForReview,
                Script {
                    base_conflicts: script([true]),
                    repair_error: Some("the repair-1 session failed"),
                    ..Script::default()
                },
            ),
            (
                Stage::RepairSession,
                "the repair-1 session failed".to_string()
            )
        );
    }

    #[test]
    fn exhausted_repairs_stop_in_a_repair_loop_round_without_a_session() {
        let (delivered, _, stages) = deliver_with_stages(
            Goal::ReadyForReview,
            Script {
                base_conflicts: script([true; MAX_REPAIRS + 1]),
                ..Script::default()
            },
        );
        assert!(cause(delivered).starts_with("repairs exhausted"));
        assert_eq!(stages.last(), Some(&Stage::RepairLoopRound));
        assert_eq!(
            stages
                .iter()
                .filter(|stage| **stage == Stage::RepairSession)
                .count(),
            MAX_REPAIRS
        );
    }

    #[test]
    fn a_refused_self_merge_stops_in_the_self_merge() {
        let (delivered, _, stages) = deliver_with_stages(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Passed, Is::Passed]),
                merges: script([Some("refused by a ruleset")]),
                ..Script::default()
            },
        );
        assert!(delivered.unwrap_err().is::<PolicyRefusal>());
        assert_eq!(
            stages,
            [
                Stage::RepairLoopRound,
                Stage::Push,
                Stage::CiWatch,
                Stage::SelfMerge,
                Stage::SelfMerge,
                Stage::RepairLoopRound,
                Stage::RepairLoopRound,
                Stage::Push,
                Stage::CiWatch,
                Stage::SelfMerge,
            ]
        );
    }

    #[test]
    fn a_base_fix_and_a_ready_pull_request_enter_their_stages() {
        let (delivered, _, stages) = deliver_with_stages(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                assessments: script([Is::Failed(&[], &["lint"]), Is::Passed]),
                base_fixes: script([None]),
                ..Script::default()
            },
        );
        assert!(delivered.is_ok());
        assert_eq!(
            stages,
            [
                Stage::RepairLoopRound,
                Stage::Push,
                Stage::CiWatch,
                Stage::BaseFix,
                Stage::RepairLoopRound,
                Stage::Push,
                Stage::CiWatch,
                Stage::PullRequestReadiness,
            ]
        );
    }
}
