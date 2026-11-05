//! Take an explicit pull request straight through Delivery and its Repair loop.

use anyhow::Context;
use chrono::Utc;

use crate::args::{self, FixArgs};
use crate::asks::{Asks, Flags};
use crate::base_fix::{BaseFix, BaseFixAsk};
use crate::child_run::ChildAsks;
use crate::config::UserConfig;
use crate::delivery::Delivery;
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::logs::{self, Work};
use crate::pull_request::PullRequest;
use crate::recovery::Continuation;
use crate::review::Reservation;
use crate::run::Ended;
use crate::session::Logs;
use crate::worktree::Worktree;

pub(crate) fn run(command: &FixArgs, config: &UserConfig, harness: &mut Choice) -> Ended {
    let mut base_fix = None;
    let outcome = (|| -> Result<_, crate::failed_run::FailedRun> {
        let launch = Git::new(std::env::current_dir().context("could not read Launch directory")?);
        let origin = launch.run(&["config", "remote.origin.url"])?;
        let repo = Repo::of_origin(&origin).context("fix needs a GitHub origin")?;
        let pr = command.pull_request.resolve(&repo)?;
        if !pr.issue.matches_origin(&origin) {
            return Err(anyhow::anyhow!("origin mismatch: {} is not in {origin}", pr.url).into());
        }
        let github = GitHub::new();
        github.fix_pull_request(&pr)?;
        let _reservation = Reservation::take(&launch, &github, &pr)?
            .context("pull request is already reserved or labelled in-progress")?;
        let snapshot = github.fix_pull_request(&pr)?;
        crate::command_limit::hold_slot(&repo);
        harness.check()?;
        logs::started(Work::FixRun(&repo), harness);
        logs::issue_branch(Some(&snapshot.branch));
        logs::pull_request(&pr.url);
        logs::issue_title(Some(snapshot.title.clone()));
        let flags = Flags {
            security_review: Some(crate::security::review::Ask::Forbid),
            ..command.flags.clone()
        };
        let mut asks = Asks::of_run(&pr.issue, &flags, config);
        asks.harness = harness.clone();
        if matches!(asks.base_fix, BaseFixAsk::Undecided { .. }) {
            asks.base_fix = BaseFixAsk::Undecided {
                retry: args::fix_retry(
                    &pr.url,
                    &Flags {
                        base_fix: Some(BaseFixAsk::Allow),
                        ..command.flags.clone()
                    },
                ),
            };
        }
        let mut repair_base = BaseFix::new(None, asks.base_fix.clone());
        let child_asks = ChildAsks::of(&asks);
        let steps = crate::processes::fix::steps()?;
        let logs = Logs::of_fix_run(&repo);
        let worktree =
            Worktree::continue_existing(&launch, &repo.name, &snapshot.branch, &snapshot.base)?;
        let mut target =
            PullRequest::on_pull_request(&pr, &snapshot.branch, &snapshot.base, github);
        let result = Delivery {
            security_review: false,
            child_asks: &child_asks,
            issue: &pr.issue,
            base: &snapshot.base,
            goal: asks.goal,
            base_fix: &mut repair_base,
            logs: &logs,
            started: Utc::now() - crate::run::STARTED_MARGIN,
            repair_cap: asks.delivery_limits.repair_cap,
            protected_paths: &asks.delivery_limits.protected_paths,
        }
        .fix_run(worktree, &mut target, &pr, steps);
        base_fix = Some(repair_base);
        result.map_err(|mut failed| {
            if let Some(recovery) = &mut failed.recovery {
                recovery.add_command_context(
                    Continuation {
                        command: args::fix_retry(&pr.url, &command.flags),
                        launch: std::env::current_dir().unwrap_or_default(),
                        base: snapshot.base.clone(),
                        branch: snapshot.branch.clone(),
                    },
                    true,
                );
                recovery.for_fix_run();
            }
            failed
        })
    })();
    Ended {
        outcome,
        base_fix: base_fix.as_ref().and_then(BaseFix::report),
        advice: base_fix.map(BaseFix::into_advice).unwrap_or_default(),
    }
}
