//! Base branch maintenance precedes the unscoped Fix Pass's PR search.
use anyhow::{Result, ensure};
use chrono::Utc;
use sha2::{Digest, Sha256};

use super::*;
use crate::ci;
use crate::github::TipCi;
use crate::issue::IssueUrl;
use crate::run::{Goal, Resolution};

pub(super) enum Outcome {
    Clear,
    Skipped(String),
    Run(Ended),
}

pub(super) fn run(
    launch: &crate::launch::Launch,
    command: &FixArgs,
    config: &UserConfig,
    harness: &mut Choice,
    started: &mut crate::command::Started,
) -> Result<Outcome> {
    let repo = &launch.repo;
    let base = launch.base.name();
    let tip = launch.base.origin_commit();
    // Share Pickup's CI reservation so overlapping hosts cannot re-run or
    // publish the same Base branch repair concurrently.
    let Some(_held) = crate::reservation::Reservation::ci_red(launch.directory.git())? else {
        return Ok(Outcome::Skipped(
            "Base branch CI is reserved on origin".into(),
        ));
    };
    let github = GitHub::new();
    // Delivery's repository-scoped adapters take an IssueUrl. No issue exists
    // for this repair: BaseBranch context must never read or close this anchor.
    let repository = IssueUrl {
        url: format!("https://github.com/{}", repo.slug()),
        owner: repo.owner.clone(),
        repo: repo.name.clone(),
        number: 0,
    };
    let checks = match github.fix_base_tip(&repository, base, tip)? {
        TipCi::Waiting => {
            return Ok(Outcome::Skipped(format!(
                "CI on {base} is pending or re-running"
            )));
        }
        TipCi::Checks(checks) => checks,
    };
    if !ci::needs_repair(&checks) {
        return Ok(Outcome::Clear);
    }
    let failed = ci::base_failures(checks);
    let branch = format!("fix-{base}-{}", ci::short(tip));
    // A local ref retains the unchanged attempt across Passes, without an
    // empty branch on origin. Hash the Base branch so slash-containing names
    // and long names cannot collide with the commit segment or ref limits.
    let unchanged = format!(
        "refs/thirdshift/base-ci-unchanged/{:x}/{tip}",
        Sha256::digest(base.as_bytes())
    );
    if !launch
        .directory
        .git()
        .run(&["for-each-ref", "--format=%(refname)", &unchanged])?
        .is_empty()
    {
        return Ok(Outcome::Skipped(format!(
            "CI-fix already found nothing to change on {base} at {tip}"
        )));
    }
    if github.base_fix_pr_exists(repo, &branch)? || launch.directory.git().on_origin(&branch)? {
        return Ok(Outcome::Skipped(format!(
            "a repair branch or pull request from {branch} already exists"
        )));
    }
    harness.check()?;
    logs::started(Work::FixRun(repo), harness);
    logs::base_branch(base);
    logs::issue_branch(Some(&branch));
    let title = format!("Fix CI on {base}: {}", ci::title_check_names(&failed));
    logs::issue_title(Some(title.clone()));
    let mut asks = Asks::of_run(&repository, &command.flags, config);
    asks.harness = harness.clone();
    let child_asks = ChildAsks::of(&asks);
    let mut repair_base = BaseFix::new(
        Some(&crate::child_run::Kind::BaseFix {
            base: base.into(),
            spec_pr: None,
        }),
        BaseFixAsk::Forbid,
    );
    let worktree = Worktree::create_fresh(launch.directory.git(), &repo.name, &branch, base)?;
    ensure!(
        worktree.head()? == tip,
        "Base branch moved before repair acquisition; retry the Fix Pass"
    );
    let logs = Logs::of_fix_run(repo);
    let mut target = PullRequest::new(&repository, &branch, base, github);
    let outcome = Delivery {
        security_review: false,
        child_asks: &child_asks,
        issue: &repository,
        base,
        goal: asks.goal,
        base_fix: &mut repair_base,
        logs: &logs,
        started: Utc::now() - crate::run::STARTED_MARGIN,
        repair_cap: asks.delivery_limits.repair_cap,
        protected_paths: &asks.delivery_limits.protected_paths,
    }
    .base_ci_fix(worktree, &mut target, &title, &failed)
    .and_then(|reached| {
        if matches!(reached.resolution, Resolution::BaseCiUnchanged { .. }) {
            launch
                .directory
                .git()
                .run(&["update-ref", &unchanged, tip])?;
        }
        Ok(reached)
    })
    .map_err(|mut failed| {
        if let Some(recovery) = &mut failed.recovery
            && let Some(pr) = target.info()
        {
            recovery.add_command_context(
                Continuation {
                    command: args::fix_retry(&pr.url, &command.flags),
                    launch: std::env::current_dir().unwrap_or_default(),
                    base: base.into(),
                    branch: branch.clone(),
                },
                true,
            );
            recovery.for_fix_run();
        }
        failed
    });
    if let Some(info) = target.info()
        && let Ok(pr) = crate::review::PullRequestUrl::parse(&info.url)
    {
        started.pull_request(&pr, &title);
    }
    if let Ok(reached) = &outcome
        && let Resolution::PullRequest {
            url,
            goal: Goal::Merged,
        } = &reached.resolution
        && let Err(error) = GitHub::new()
            .completion()
            .close_base_ci_issues(repo, base, url)
    {
        crate::progress::warn(
            &error,
            format_args!("could not close CI-red issues on {base}"),
        );
    }
    Ok(Outcome::Run(Ended {
        outcome,
        base_fix: None,
        advice: Vec::new(),
    }))
}
