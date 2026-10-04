//! The Repair loop: what keeps a Delivery's ready pull request mergeable
//! and its CI green, within its Repair and upstream-move budgets, and, in a
//! Merge run, merges it, going round again after a failed merge until it
//! merges or is a Policy refusal. It reaches the Issue branch, CI, the
//! Repair sessions, the Base fix and the pull request only through
//! [`Outside`].

use anyhow::{Result, bail};

use crate::ci::{self, Ci, FailedChecks};
use crate::failed_run::PolicyRefusal;
use crate::run::Goal;

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

    /// Merge the Base branch, fetched from origin, into the Issue branch,
    /// returning the merge left pending if it conflicted.
    fn merge_base_branch(&mut self) -> Result<Option<Self::Pending>>;
    /// Merge the new commits on origin, as last fetched, into the Issue
    /// branch, returning the merge left pending if it conflicted.
    fn merge_new_commits(&mut self) -> Result<Option<Self::Pending>>;
    /// Fail unless the merge `pending` was finished.
    fn ensure_merged(&mut self, pending: &Self::Pending) -> Result<()>;
    /// Push the Issue branch.
    fn push(&mut self) -> Result<()>;
    /// The Issue branch's head commit.
    fn head(&mut self) -> Result<String>;
    /// The Base branch commit the Issue branch last merged in.
    fn merged_base_commit(&mut self) -> Result<String>;
    /// Whether the Base branch on origin has commits the Issue branch has
    /// not merged yet.
    fn base_branch_moved(&mut self) -> Result<bool>;
    /// The commits on the Issue branch on origin, oldest first, that the
    /// local one does not have yet.
    fn new_commits_on_origin(&mut self) -> Result<Vec<String>>;
    /// The name of the Issue branch on origin, as in `origin/issue-7`.
    fn upstream(&mut self) -> String;

    /// Watch CI on `head`, comparing its failed checks with those on
    /// `base_commit`, if given.
    fn watch(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci>;
    /// The Check re-run of `head`, after a CI-fix Repair given `failed` left
    /// it unchanged: `None` if there is none to watch.
    fn rerun(
        &mut self,
        head: &str,
        base_commit: Option<&str>,
        failed: &FailedChecks,
    ) -> Result<Option<Ci>>;

    /// Start the Repair session `kind`, as in `repair-1`, for `repair`.
    fn repair(&mut self, kind: &str, repair: Repair) -> Result<()>;

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    fn sees_inherited_failures(&mut self) -> bool;
    /// Take the Base fix for the Inherited failures in `failed`, which also
    /// fail on the Base branch at `base_commit`, returning once the Run is to
    /// go round again.
    fn base_fix(&mut self, base_commit: &str, failed: &FailedChecks) -> Result<()>;

    /// Fail unless the pull request is open, ready for review and mergeable.
    fn ensure_pr_ready_and_mergeable(&mut self) -> Result<()>;
    /// Merge the pull request at `head`.
    fn merge(&mut self, head: &str) -> Result<()>;

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
/// the merge.
pub(super) fn take_to_goal(outside: &mut impl Outside, base: &str, goal: Goal) -> Result<()> {
    let mut repair_loop = RepairLoop {
        outside,
        base,
        goal,
        budgets: Budgets::default(),
    };
    let mut watched = repair_loop.rounds()?;
    loop {
        let outside = &mut *repair_loop.outside;
        outside.ensure_pr_ready_and_mergeable()?;
        if outside.interrupt_requested() {
            bail!("interrupted");
        }
        if goal == Goal::ReadyForReview {
            return Ok(());
        }
        outside.progress(format!("merging the PR into {base}"));
        let Err(error) = outside.merge(&watched) else {
            return Ok(());
        };
        outside.progress(format!("the merge failed: {error:#}"));
        match repair_loop.round_after_failed_merge(&watched)? {
            Round::NewHead(head) => watched = head,
            Round::NothingToFix => {
                // Only a PR still ready and mergeable is left ready.
                repair_loop.outside.ensure_pr_ready_and_mergeable()?;
                return Err(error.context(PolicyRefusal));
            }
        }
    }
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
    /// Counts the Repair about to start, returning its kind, `repair-<n>`,
    /// and its progress line, or fails if it would be one too many.
    fn next_repair(&mut self, cause: &str) -> Result<(String, String)> {
        if self.repairs == MAX_REPAIRS {
            bail!("repairs exhausted: {cause}");
        }
        self.repairs += 1;
        let line = format!("{cause}; starting Repair {} of {MAX_REPAIRS}", self.repairs);
        Ok((format!("repair-{}", self.repairs), line))
    }

    /// Counts a round taken because `upstream` moved, returning `why` as its
    /// progress line, or fails if it would be one too many.
    fn count_upstream_move(&mut self, upstream: &str, why: String) -> Result<String> {
        if self.upstream_moves == MAX_UPSTREAM_MOVES {
            bail!("{upstream} kept moving: merged it again {MAX_UPSTREAM_MOVES} times");
        }
        self.upstream_moves += 1;
        Ok(why)
    }

    /// Counts a round taken because `origin/<base>` moved `when`, returning
    /// its progress line.
    fn count_base_move(&mut self, base: &str, when: &str) -> Result<String> {
        self.count_upstream_move(
            &format!("origin/{base}"),
            format!("origin/{base} moved {when}; merging it again"),
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
        let (kind, line) = self.budgets.next_repair(cause)?;
        self.outside.progress(line);
        self.outside.repair(&kind, repair)
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
    /// meanwhile. Green or absent CI also goes round again if the Base branch
    /// moved while CI ran, or, in a Merge run, if Foreign commits arrived.
    /// A red check that also fails on the Base branch commit the head last
    /// merged in is an Inherited failure, which no Repair is started for: the
    /// CI-fix Repair is given the branch's own failures, and when there are
    /// none, the loop goes round again if the Base branch moved since, counted
    /// as a Base move, and otherwise once its Base fix has returned, or fails
    /// with the Base fix's cause. A Run that sees no Inherited failures
    /// compares with no Base branch commit.
    /// If, after a CI-fix Repair and the Base branch merged again, the head
    /// is still the one whose CI failed, it gets its Check re-run instead of
    /// a watch, whatever the Repair concluded. CI then
    /// green, or red only on Inherited failures, is taken as from any watch.
    /// Returns the head commit whose CI was last watched and found green or
    /// absent. Fails with a Declined CI fix if that Check re-run leaves a
    /// check of the branch's own red, or there can be none, and once a Repair
    /// beyond `MAX_REPAIRS`, or a round beyond `MAX_UPSTREAM_MOVES`, would be
    /// needed.
    fn rounds(&mut self) -> Result<String> {
        let base = self.base;
        // The head the last CI-fix Repair was given, with its failed checks.
        let mut handed_to_repair: Option<(String, FailedChecks)> = None;
        loop {
            if self.goal == Goal::Merged {
                self.take_in_foreign_commits()?;
            }
            if let Some(pending) = self.outside.merge_base_branch()? {
                self.repair("conflict", Repair::Conflict(Upstream::BaseBranch))?;
                self.outside.ensure_merged(&pending)?;
                continue;
            }
            self.outside.push()?;
            let head = self.outside.head()?;
            let base_commit = self.outside.merged_base_commit()?;
            let compared_with = self
                .outside
                .sees_inherited_failures()
                .then_some(base_commit.as_str());
            let unchanged = handed_to_repair
                .take()
                .filter(|(handed, _)| *handed == head);
            let ci = match unchanged {
                None => self.outside.watch(&head, compared_with)?,
                // A Declined CI fix, unless the head's one Check re-run turns
                // the branch's own checks green: it gets no second Repair.
                Some((_, failed)) => match self.outside.rerun(&head, compared_with, &failed)? {
                    Some(ci) if !ci.has_own_failures() => ci,
                    _ => bail!(
                        "CI red on {} and the Repair found nothing to fix on the branch",
                        ci::short(&head)
                    ),
                },
            };
            match ci {
                Ci::Absent | Ci::Passed => {
                    if self.outside.base_branch_moved()? {
                        self.count_base_move("while CI ran")?;
                    } else if self.goal == Goal::ReadyForReview
                        || self.outside.new_commits_on_origin()?.is_empty()
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
                        self.outside.base_fix(&base_commit, &failed)?;
                        continue;
                    }
                    self.repair("CI red", Repair::CiFix(&failed))?;
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
        let upstream = self.outside.upstream();
        loop {
            let foreign = self.outside.new_commits_on_origin()?;
            if foreign.is_empty() {
                return Ok(());
            }
            let own_head = self.outside.head()?;
            let line = self.budgets.count_upstream_move(
                &upstream,
                format!("{upstream} has new commits; merging them in"),
            )?;
            self.outside.progress(line);
            for sha in &foreign {
                self.outside
                    .progress(format!("merging new commit {sha} from {upstream}"));
            }
            if let Some(pending) = self.outside.merge_new_commits()? {
                self.repair(
                    &format!("conflict with new commits on {upstream}"),
                    Repair::Conflict(Upstream::IssueBranch),
                )?;
                self.outside.ensure_merged(&pending)?;
            }
            self.repair(
                &format!("new commits on {upstream} to review"),
                Repair::Review { from: &own_head },
            )?;
        }
    }

    /// Go round again after a merge of `watched` failed, counting a Base
    /// branch that moved since as an upstream move. GitHub's error text is never
    /// consulted.
    fn round_after_failed_merge(&mut self, watched: &str) -> Result<Round> {
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
    use crate::github::{Check, CheckState};

    /// What the Repair loop did outside itself, in the order it did it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Did {
        /// Merged the Base branch in.
        MergeBase,
        /// Merged the new commits on origin in.
        MergeNewCommits,
        /// Checked the pending merge of this upstream was finished.
        CheckMerged(&'static str),
        Push,
        /// Watched CI on this head, compared with this Base branch commit.
        Watch(String, Option<String>),
        /// Gave this head its Check re-run of these checks of its own.
        Rerun(String, Vec<String>),
        /// Started the Repair session of this kind for this Repair.
        Repair(String, Started),
        /// Took the Base fix for these Inherited failures, at this Base
        /// branch commit.
        BaseFix(String, Vec<String>),
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
                        state: CheckState::Failed,
                        url: None,
                        job: None,
                    })
                    .collect()
            };
            match self {
                Is::Absent => Ci::Absent,
                Is::Passed => Ci::Passed,
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
    /// unmoved, origin without new commits and a Base fix returns, unless
    /// scripted otherwise.
    #[derive(Default)]
    struct Script {
        heads: VecDeque<&'static str>,
        base_commits: VecDeque<&'static str>,
        /// For each merge of the Base branch, whether it conflicts.
        base_conflicts: VecDeque<bool>,
        /// For each merge of new commits on origin, whether it conflicts.
        new_commit_conflicts: VecDeque<bool>,
        base_moved: VecDeque<bool>,
        new_commits: VecDeque<&'static [&'static str]>,
        watches: VecDeque<Is>,
        reruns: VecDeque<Option<Is>>,
        /// For each Base fix, the cause it fails with, if it does.
        base_fixes: VecDeque<Option<&'static str>>,
        /// For each merge, the error it fails with, if it does.
        merges: VecDeque<Option<&'static str>>,
        sees_no_inherited_failures: bool,
        interrupted: bool,
    }

    /// The outside world as `script` answers it, with what the loop did there
    /// recorded in `did`.
    struct Scripted {
        script: Script,
        did: Vec<Did>,
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

        fn merge_base_branch(&mut self) -> Result<Option<&'static str>> {
            self.did.push(Did::MergeBase);
            let conflicted = self.script.base_conflicts.pop_front().unwrap_or(false);
            Ok(conflicted.then_some("origin/main"))
        }

        fn merge_new_commits(&mut self) -> Result<Option<&'static str>> {
            self.did.push(Did::MergeNewCommits);
            let conflicted = self
                .script
                .new_commit_conflicts
                .pop_front()
                .unwrap_or(false);
            Ok(conflicted.then_some("origin/issue-7"))
        }

        fn ensure_merged(&mut self, pending: &&'static str) -> Result<()> {
            self.did.push(Did::CheckMerged(pending));
            Ok(())
        }

        fn push(&mut self) -> Result<()> {
            self.did.push(Did::Push);
            Ok(())
        }

        fn head(&mut self) -> Result<String> {
            Ok(sticky(&mut self.script.heads, "head"))
        }

        fn merged_base_commit(&mut self) -> Result<String> {
            if self.script.base_commits.is_empty() {
                return Ok("b1".to_string());
            }
            Ok(sticky(&mut self.script.base_commits, "Base branch commit"))
        }

        fn base_branch_moved(&mut self) -> Result<bool> {
            Ok(self.script.base_moved.pop_front().unwrap_or(false))
        }

        fn new_commits_on_origin(&mut self) -> Result<Vec<String>> {
            let commits = self.script.new_commits.pop_front().unwrap_or_default();
            Ok(commits.iter().map(|sha| sha.to_string()).collect())
        }

        fn upstream(&mut self) -> String {
            "origin/issue-7".to_string()
        }

        fn watch(&mut self, head: &str, base_commit: Option<&str>) -> Result<Ci> {
            self.did
                .push(Did::Watch(head.to_string(), base_commit.map(String::from)));
            let is = self.script.watches.pop_front().expect("no watch scripted");
            Ok(is.ci())
        }

        fn rerun(
            &mut self,
            head: &str,
            _base_commit: Option<&str>,
            failed: &FailedChecks,
        ) -> Result<Option<Ci>> {
            self.did
                .push(Did::Rerun(head.to_string(), names(&failed.own)));
            let is = self.script.reruns.pop_front().expect("no re-run scripted");
            Ok(is.map(|is| is.ci()))
        }

        fn repair(&mut self, kind: &str, repair: Repair) -> Result<()> {
            let started = match repair {
                Repair::Conflict(upstream) => Started::Conflict(upstream),
                Repair::CiFix(failed) => {
                    Started::CiFix(names(&failed.own), names(&failed.inherited))
                }
                Repair::Review { from } => Started::Review(from.to_string()),
            };
            self.did.push(Did::Repair(kind.to_string(), started));
            Ok(())
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

        fn ensure_pr_ready_and_mergeable(&mut self) -> Result<()> {
            self.did.push(Did::CheckPr);
            Ok(())
        }

        fn merge(&mut self, head: &str) -> Result<()> {
            self.did.push(Did::Merge(head.to_string()));
            match self.script.merges.pop_front().expect("no merge scripted") {
                Some(error) => Err(anyhow!(error)),
                None => Ok(()),
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
        let mut outside = Scripted {
            script,
            did: Vec::new(),
        };
        let delivered = take_to_goal(&mut outside, "main", goal);
        (delivered, outside.did)
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
    fn watch(head: &str) -> Did {
        Did::Watch(head.to_string(), Some("b1".to_string()))
    }

    /// The Repair session `repair-<n>` for `started`.
    fn repair(n: usize, started: Started) -> Did {
        Did::Repair(format!("repair-{n}"), started)
    }

    /// A CI-fix Repair given the branch's own failures `own`, and the
    /// Inherited failures `inherited`.
    fn ci_fix(own: &[&str], inherited: &[&str]) -> Started {
        let names = |checks: &[&str]| checks.iter().map(|name| name.to_string()).collect();
        Started::CiFix(names(own), names(inherited))
    }

    /// A round from the merge of the Base branch to the watch of `head`.
    fn round(head: &str) -> [Did; 3] {
        [Did::MergeBase, Did::Push, watch(head)]
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
            .filter_map(|did| match did {
                Did::Line(line) => Some(line.as_str()),
                _ => None,
            })
            .collect()
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
                    watches: script([is]),
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
                new_commits: script([&["f1"][..], &["f1"]]),
                watches: script([Is::Passed]),
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
                watches: script([Is::Passed]),
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
                watches: script([Is::Failed(&["test"], &["lint"]), Is::Passed]),
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
                ],
                &round("h2"),
                &[Did::CheckPr],
            ]
            .concat()
        );
    }

    /// What the loop does up to the Check re-run of `h1`, after a CI-fix
    /// Repair given its failed `test` left it unchanged.
    fn up_to_the_rerun() -> Vec<Did> {
        [
            &round("h1")[..],
            &[
                line("CI red; starting Repair 1 of 5"),
                repair(1, ci_fix(&["test"], &[])),
                Did::MergeBase,
                Did::Push,
                Did::Rerun("h1".to_string(), vec!["test".to_string()]),
            ],
        ]
        .concat()
    }

    #[test]
    fn an_unchanged_head_after_a_ci_fix_repair_gets_its_check_re_run_and_green_reaches_the_goal() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1"]),
                watches: script([Is::Failed(&["test"], &[])]),
                reruns: script([Some(Is::Passed)]),
                merges: script([None]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(did, [up_to_the_rerun(), merge("h1").into()].concat());
    }

    #[test]
    fn a_check_re_run_red_only_on_inherited_failures_is_taken_as_from_any_watch() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                watches: script([Is::Failed(&["test"], &[]), Is::Passed]),
                reruns: script([Some(Is::Failed(&[], &["lint"]))]),
                ..Script::default()
            },
        );

        delivered.unwrap();
        assert_eq!(
            did,
            [
                up_to_the_rerun(),
                vec![
                    line("Inherited failures (also failing on main at b1): lint"),
                    Did::BaseFix("b1".to_string(), vec!["lint".to_string()]),
                ],
                round("h1").into(),
                vec![Did::CheckPr],
            ]
            .concat()
        );
    }

    #[test]
    fn a_check_re_run_red_on_its_own_checks_or_impossible_is_a_declined_ci_fix() {
        for rerun in [Some(Is::Failed(&["test"], &[])), None] {
            let (delivered, did) = deliver(
                Goal::Merged,
                Script {
                    heads: script(["h1"]),
                    watches: script([Is::Failed(&["test"], &[])]),
                    reruns: script([rerun]),
                    ..Script::default()
                },
            );

            assert_eq!(
                cause(delivered),
                "CI red on h1 and the Repair found nothing to fix on the branch"
            );
            assert_eq!(did, up_to_the_rerun());
        }
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
                watches: script([Is::Failed(&[], &["test"]), Is::Passed]),
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
                    Did::Watch("h2".to_string(), Some("b2".to_string())),
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
                watches: script([Is::Failed(&[], &["test"]), Is::Passed]),
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
                watches: script([Is::Failed(&[], &["test"])]),
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
    fn a_run_that_sees_no_inherited_failures_watches_ci_with_no_base_branch_commit() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1"]),
                watches: script([Is::Passed]),
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
                Did::Watch("h1".to_string(), None),
                Did::CheckPr
            ]
        );
    }

    #[test]
    fn the_base_branch_moving_while_ci_ran_goes_round_again_as_a_base_move() {
        let (delivered, did) = deliver(
            Goal::ReadyForReview,
            Script {
                heads: script(["h1", "h2"]),
                base_moved: script([true]),
                watches: script([Is::Passed, Is::Passed]),
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
            Did::MergeNewCommits,
            line(&format!(
                "new commits on origin/issue-7 to review; starting Repair {n} of 5"
            )),
            repair(n, Started::Review(own_head.to_string())),
        ]
    }

    #[test]
    fn a_merge_run_merges_foreign_commits_in_and_reviews_them_while_origin_has_more() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2", "h3"]),
                new_commits: script([&["f1", "f2"][..], &["f3"]]),
                watches: script([Is::Passed]),
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
                    Did::MergeNewCommits,
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
    fn foreign_commits_that_arrive_while_ci_ran_go_round_again() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h1", "h2"]),
                new_commits: script([&[][..], &["f1"], &["f1"]]),
                watches: script([Is::Passed, Is::Passed]),
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
                heads: script(["h1", "h2"]),
                new_commits: script([&["f1"][..]]),
                new_commit_conflicts: script([true]),
                watches: script([Is::Passed]),
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
                    Did::MergeNewCommits,
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
                heads: script(["h0", "h1", "h2", "h3", "h4"]),
                new_commits: script([&["f1"][..]]),
                base_conflicts: script([true]),
                watches: script([
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
        assert_eq!(did.last(), Some(&watch("h4")));
    }

    #[test]
    fn base_moves_and_foreign_commits_share_one_upstream_move_budget() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h0", "h1"]),
                new_commits: script([&["f1"][..]]),
                base_moved: script([true; 5]),
                watches: script((0..5).map(|_| Is::Passed)),
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
    fn a_failed_merge_goes_round_again_counting_a_base_move_and_retries_on_the_new_head() {
        let (delivered, did) = deliver(
            Goal::Merged,
            Script {
                heads: script(["h1", "h2"]),
                base_moved: script([false, true, false]),
                watches: script([Is::Passed, Is::Passed]),
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
                watches: script((0..6).map(|_| Is::Passed)),
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
                watches: script([Is::Passed, Is::Passed]),
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
                watches: script([Is::Passed, Is::Passed]),
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
                watches: script([Is::Passed]),
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
                    format!("repair-{n}"),
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
    fn upstream_moves_of_the_base_or_the_issue_branch_share_one_budget() {
        let mut budgets = Budgets::default();
        for _ in 1..MAX_UPSTREAM_MOVES {
            assert_eq!(
                budgets.count_base_move("main", "while CI ran").unwrap(),
                "origin/main moved while CI ran; merging it again"
            );
        }
        assert_eq!(
            budgets
                .count_upstream_move("origin/issue-7", "new commits".to_string())
                .unwrap(),
            "new commits"
        );

        let error = budgets.count_base_move("main", "while CI ran").unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/main kept moving: merged it again {MAX_UPSTREAM_MOVES} times")
        );
        let error = budgets
            .count_upstream_move("origin/issue-7", "new commits".to_string())
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
