//! Continue a delivered Review head through the Fix run's shared Self-merge.

use super::*;
use crate::failed_run::FailedRun;
use crate::run::Reached;
use crate::worktree::Worktree;

pub(super) struct SelfMerge<'a> {
    pub pr: &'a PullRequestUrl,
    pub snapshot: &'a ReviewPullRequest,
    pub choice: &'a Choice,
    pub logs: &'a session::Logs,
    /// The Review run's command, frozen with its merge choice.
    pub retry: &'a str,
    pub limits: &'a Limits,
    pub started: chrono::DateTime<Utc>,
}

impl SelfMerge<'_> {
    pub(super) fn run(&self, worktree: Worktree) -> Result<Reached, FailedRun> {
        let Self {
            pr,
            snapshot,
            choice,
            logs,
            retry,
            limits,
            started,
        } = *self;
        let child_asks = ChildAsks {
            harness: choice.clone(),
            ..ChildAsks::default()
        };
        let mut base_fix = BaseFix::new(None, crate::base_fix::BaseFixAsk::Forbid);
        let mut target =
            PullRequest::on_pull_request(pr, &snapshot.branch, &snapshot.base, GitHub::new());
        target.require_unstacked_base();
        crate::delivery::Delivery {
            security_review: false,
            child_asks: &child_asks,
            issue: &pr.issue,
            base: &snapshot.base,
            goal: Goal::Merged,
            base_fix: &mut base_fix,
            logs,
            started,
            repair_cap: limits.repair_cap,
            protected_paths: &limits.protected_paths,
        }
        .fix_run(worktree, &mut target, pr, crate::processes::fix::steps()?)
        .map_err(|mut failed| {
            if let Some(recovery) = &mut failed.recovery {
                recovery.add_command_context(
                    crate::recovery::Continuation {
                        command: retry.into(),
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
}
