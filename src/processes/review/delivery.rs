//! Publish Review fixes and gate readiness on CI for the exact delivered head.

use super::*;
use crate::ci::{self, Ci, FailedChecks, Watch};
use crate::github::review::ReviewInput;
use crate::poll;
use crate::pull_request::PullRequest;

pub(super) struct Delivery<'a> {
    pub git: &'a Git,
    pub github: &'a GitHub,
    pub pr: &'a PullRequestUrl,
    pub snapshot: &'a ReviewPullRequest,
    pub mode: Mode,
    pub merge: bool,
    pub input: &'a ReviewInput,
}

impl Delivery<'_> {
    pub fn run(
        &self,
        findings: &str,
        repair_cap: usize,
        mut repair: impl FnMut(session::Kind, &FailedChecks, &ReviewPullRequest) -> Result<String>,
    ) -> Result<()> {
        let Self {
            git,
            github,
            pr,
            snapshot,
            mode,
            input,
            ..
        } = *self;
        let mut stack = String::new();
        if !git.succeeds(&["merge-base", "--is-ancestor", &snapshot.base_head, "HEAD"])? {
            for fix in github.review_base_fixes(pr, snapshot)? {
                if git.succeeds(&["merge-base", "--is-ancestor", &fix, &snapshot.base_head])?
                    && !git.succeeds(&["merge-base", "--is-ancestor", &fix, "HEAD"])?
                {
                    stack = format!(
                        "\n\nThe base branch {} received Review run fixes. This pull request needs updating before merge.",
                        snapshot.base
                    );
                    break;
                }
            }
        }
        let mut pushed = None;
        let mut ci_report = String::new();
        let readiness = match mode {
            Mode::RecommendOnly => {
                unchanged_checkout(git, snapshot)?;
                Ok(())
            }
            Mode::Address => self.address(&mut pushed, &mut ci_report, repair_cap, &mut repair),
        };
        let current = if mode == Mode::RecommendOnly {
            unchanged(github, pr, snapshot)?;
            None
        } else {
            Some(compatible(github, pr, snapshot)?)
        };
        let mut marker = serde_json::json!({"reviewed_head": snapshot.head});
        let status = if let Some(head) = &pushed {
            marker["pushed_head"] = head.clone().into();
            format!("Pushed fixes to {} at {head}.", snapshot.branch)
        } else if mode == Mode::RecommendOnly {
            "--recommend-only; nothing pushed.".into()
        } else if current.as_ref().is_some_and(|pr| pr.head != snapshot.head) {
            "The branch moved; no fixes were pushed. Proposed fixes remain in the local worktree."
                .into()
        } else if readiness.is_err() && git.run(&["rev-parse", "HEAD"])? != snapshot.head {
            "Could not push fixes; proposed fixes remain in the local worktree.".into()
        } else {
            "No fixes to push.".into()
        };
        let spec = if input.body_alone {
            "Spec axis used the pull request body alone; no closing issue was linked."
        } else {
            "Spec axis used the linked closing issues."
        };
        // Retain the failing-check evidence in both the posted result and the
        // error used by the Command record and Run notification.
        let delivered_head = pushed.as_deref().unwrap_or(&snapshot.head);
        let readiness = if mode == Mode::Address {
            self.unchanged_head(delivered_head).and(readiness)
        } else {
            readiness
        };
        let readiness = if ci_report.is_empty() {
            readiness
        } else {
            readiness.with_context(|| ci_report.clone())
        };
        let failure = match &readiness {
            Ok(()) => String::new(),
            Err(error) => format!("\n\n### Delivery failed\n\n{error:#}"),
        };
        github.comment_review(
            pr,
            &format!(
                "Review run. {status} {spec}{stack}\n\n{findings}{ci_report}{failure}\n\n<!-- thirdshift-review {marker} -->\n"
            ),
        )?;
        readiness?;
        if mode == Mode::Address {
            self.unchanged_head(delivered_head)?;
        }
        Ok(())
    }

    fn address(
        &self,
        pushed: &mut Option<String>,
        report: &mut String,
        repair_cap: usize,
        repair: &mut impl FnMut(session::Kind, &FailedChecks, &ReviewPullRequest) -> Result<String>,
    ) -> Result<()> {
        let Self {
            git,
            github,
            pr,
            snapshot,
            ..
        } = *self;
        *pushed = self.push_fixes(snapshot)?;
        let mut expected = snapshot.clone();
        if let Some(head) = pushed.as_ref() {
            self.wait_for_pushed_head(&expected, head)?;
            expected.head = head.clone();
        }
        // Review never merges the author's branch or stack. Compare with the
        // Base commit this branch actually contains, not a newer Base tip.
        let base = git.run(&["merge-base", &expected.head, &snapshot.base_head])?;
        let mut watch = Watch::new(&pr.issue);
        let mut repairs = 0;
        loop {
            unchanged(github, pr, &expected)?;
            let ci = watch
                .assess(
                    &expected.head,
                    Some(&base),
                    &mut || unchanged(github, pr, &expected).map(|()| Some(expected.head.clone())),
                    &mut |_| Ok(false),
                )
                .with_context(|| format!("CI watch on {} failed", expected.head))?;
            match ci {
                Ci::Absent | Ci::Passed => {
                    self.require_mergeable(&expected.head)?;
                    report.push_str(&format!(
                        "\n\n### CI passed or absent at {}\n",
                        expected.head
                    ));
                    return Ok(());
                }
                Ci::Superseded => bail!("pull request changed during Review run CI watch"),
                Ci::Failed(failed) => {
                    report.push_str(&format!(
                        "\n\n### CI red at {}\n\n{}{}\nInherited failures on {} at {}:\n{}",
                        expected.head,
                        ci::check_list(&failed.own),
                        ci::check_list(&failed.inherited),
                        snapshot.base,
                        base,
                        ci::check_list(&failed.on_base),
                    ));
                    ensure!(
                        !failed.own.is_empty(),
                        "CI red on {}, which also fails on {} at {}; fix {} first",
                        ci::check_names(&failed.inherited),
                        snapshot.base,
                        ci::short(&base),
                        snapshot.base,
                    );
                    ensure!(repairs < repair_cap, "repairs exhausted: CI red");
                    repairs += 1;
                    progress::step(format!("CI red; starting Repair {repairs} of {repair_cap}"));
                    let entry = repair(session::Kind::Repair(repairs), &failed, &expected)?;
                    unchanged(github, pr, &expected)?;
                    report.push_str(&format!("\n\n### Repair {repairs}\n\n{entry}"));
                    let next = self.push_fixes(&expected)?;
                    watch.after_repair(expected.head.clone(), failed);
                    if let Some(head) = next {
                        *pushed = Some(head.clone());
                        self.wait_for_pushed_head(&expected, &head)?;
                        expected.head = head;
                    }
                }
            }
        }
    }

    fn push_fixes(&self, snapshot: &ReviewPullRequest) -> Result<Option<String>> {
        let git = self.git;
        let head = git.run(&["rev-parse", "HEAD"])?;
        ensure!(
            git.run(&["branch", "--show-current"])? == snapshot.branch
                && git
                    .run(&["status", "--porcelain", "--untracked-files=no"])?
                    .is_empty()
                && git.succeeds(&["merge-base", "--is-ancestor", &snapshot.head, &head])?,
            "review fixes must be committed on the pull request's captured branch without rewriting history"
        );
        unchanged(self.github, self.pr, snapshot)?;
        if head == snapshot.head {
            return Ok(None);
        }
        self.require_mergeable(&snapshot.head)?;
        // A matching lease stays a fast-forward: the captured head is an ancestor.
        // It also catches a receive-time rewind that a plain push would overwrite.
        git.push_checked(&snapshot.branch, &head, Some(&snapshot.head), || {
            compatible(self.github, self.pr, snapshot).map(|_| ())
        })?;
        Ok(Some(head))
    }

    fn unchanged_head(&self, head: &str) -> Result<()> {
        if self.merge {
            let current = compatible(self.github, self.pr, self.snapshot)?;
            if current.head != head {
                return Err(crate::failed_run::MergeHold {
                    reason: format!("head moved after review; expected {head}, actual {}; the PR stays ready for review", current.head),
                }.into());
            }
            return Ok(());
        }
        let expected = ReviewPullRequest {
            head: head.into(),
            ..self.snapshot.clone()
        };
        unchanged(self.github, self.pr, &expected)
    }

    /// GitHub may still serve the pre-push head briefly after a successful push.
    /// Only that head is tolerated; eligibility and branch names stay guarded.
    fn wait_for_pushed_head(&self, snapshot: &ReviewPullRequest, head: &str) -> Result<()> {
        let deadline = std::time::Instant::now() + poll::grace_period();
        let mut waiting_line = poll::WaitingLine::new();
        let mut backoff = poll::Backoff::new();
        loop {
            let current = compatible(self.github, self.pr, snapshot)?;
            if current.head == head {
                return Ok(());
            }
            ensure!(
                current.head == snapshot.head,
                "pull request changed during Review run: head moved; expected {head} (or pre-push {}), actual {}",
                snapshot.head,
                current.head,
            );
            let now = std::time::Instant::now();
            ensure!(
                now < deadline,
                "timed out waiting for GitHub to report pushed head: expected {head}, actual {}",
                current.head,
            );
            if waiting_line.should_report(()) {
                progress::step(format!("waiting for GitHub to report pushed head {head}"));
            }
            poll::sleep(backoff.observe(current.head, now).min(deadline - now))?;
        }
    }

    /// Delegate to the pull-request owner's Review readiness for `head`.
    fn require_mergeable(&self, head: &str) -> Result<()> {
        PullRequest::on_pull_request(
            self.pr,
            &self.snapshot.branch,
            &self.snapshot.base,
            self.github.clone(),
        )
        .ensure_review_ready(head)
    }
}
