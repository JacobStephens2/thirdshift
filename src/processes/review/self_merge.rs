//! Continue a delivered Review head through the Fix run's shared Self-merge.

use super::*;
use crate::failed_run::FailedRun;
use crate::run::Reached;
use crate::worktree::Worktree;

pub(super) fn run(
    worktree: Worktree,
    pr: &PullRequestUrl,
    snapshot: &ReviewPullRequest,
    choice: &Choice,
    flags: &Flags,
    limits: &Limits,
    started: chrono::DateTime<Utc>,
) -> Result<Reached, FailedRun> {
    let child_asks = ChildAsks {
        harness: choice.clone(),
        ..ChildAsks::default()
    };
    let mut base_fix = BaseFix::new(None, crate::base_fix::BaseFixAsk::Forbid);
    let mut target =
        PullRequest::on_pull_request(pr, &snapshot.branch, &snapshot.base, GitHub::new());
    target.require_unstacked_base();
    let logs = session::Logs::of_process_review(&pr.issue.repo(), 4);
    crate::delivery::Delivery {
        security_review: false,
        child_asks: &child_asks,
        issue: &pr.issue,
        base: &snapshot.base,
        goal: Goal::Merged,
        base_fix: &mut base_fix,
        logs: &logs,
        started,
        repair_cap: limits.repair_cap,
        protected_paths: &limits.protected_paths,
    }
    .fix_run(worktree, &mut target, pr, crate::processes::fix::steps()?)
    .map_err(|mut failed| {
        if let Some(recovery) = &mut failed.recovery {
            recovery.add_command_context(
                crate::recovery::Continuation {
                    command: crate::args::review_retry(&pr.url, flags),
                    launch: std::env::current_dir().unwrap_or_default(),
                    base: snapshot.base.clone(),
                    branch: snapshot.branch.clone(),
                },
                true,
            );
        }
        failed
    })
}
