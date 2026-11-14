//! Select or target a pull request, then enter Delivery and its Repair loop.

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
use crate::run::{Ended, Goal};
use crate::session::Logs;
use crate::worktree::Worktree;

mod base_ci;

pub(crate) enum Target {
    PullRequest(crate::review::PullRequestInput),
    Pass {
        base: Option<String>,
        author: Option<String>,
    },
}

pub(crate) fn run(
    command: &FixArgs,
    config: &UserConfig,
    harness: &mut Choice,
    started: &mut crate::command::Started,
) -> crate::command::Ending {
    let result = (|| -> anyhow::Result<_> {
        if let Target::Pass { base, author } = &command.target {
            let launch = match crate::launch::start(
                base.as_deref(),
                config.command_limit,
                crate::command_limit::Sharing::UpToTheLimit,
            )? {
                crate::launch::Start::Clear(launch) => launch,
                crate::launch::Start::NotAdmitted(running) => {
                    return Ok(skipped(&running.repo, running.to_string()));
                }
            };
            let github = GitHub::new();
            let base = launch.base.name();
            if author.is_none() {
                match base_ci::run(&launch, command, config, harness, started)? {
                    base_ci::Outcome::Clear => {}
                    base_ci::Outcome::Skipped(reason) => {
                        logs::base_ci_skipped(&launch.repo, &reason);
                    }
                    base_ci::Outcome::Run(ended) => {
                        return Ok(crate::command::Ending::Run(ended));
                    }
                }
            }
            let mut passed_over = Vec::new();
            for pr in github.fix_candidates(&launch.repo, base, author.as_deref())? {
                let Some(branch) =
                    github.fix_candidate_branch(&pr, base, author.as_deref(), false)?
                else {
                    continue;
                };
                if let Some(path) = Worktree::occupying_checkout(launch.directory.git(), &branch)? {
                    let reason = format!(
                        "passed over {}: head branch {} is checked out at {}",
                        pr.url,
                        branch,
                        path.display()
                    );
                    crate::progress::step(&reason);
                    passed_over.push(reason);
                    continue;
                }
                let Some(held) = Reservation::take(launch.directory.git(), &github, &pr)? else {
                    continue;
                };
                if github
                    .fix_candidate_branch(&pr, base, author.as_deref(), true)?
                    .is_none()
                {
                    continue;
                }
                let selected = FixArgs {
                    target: Target::PullRequest(crate::review::PullRequestInput::Url(pr)),
                    flags: command.flags.clone(),
                };
                return Ok(crate::command::Ending::Run(targeted(
                    &selected,
                    config,
                    harness,
                    started,
                    Some(held),
                )));
            }
            let mut reason = format!(
                "no eligible pull request targeting {base} on {}",
                launch.repo.slug()
            );
            if !passed_over.is_empty() {
                reason.push_str(&format!("; {}", passed_over.join("; ")));
            }
            return Ok(skipped(&launch.repo, reason));
        }
        Ok(crate::command::Ending::Run(targeted(
            command, config, harness, started, None,
        )))
    })();
    result.unwrap_or_else(|error| {
        crate::command::Ending::Run(Ended {
            outcome: Err(error.into()),
            base_fix: None,
            advice: Vec::new(),
        })
    })
}

fn skipped(repo: &Repo, reason: String) -> crate::command::Ending {
    logs::skipped(logs::Pass::Fix, repo, &reason);
    crate::command::Ending::Skipped(crate::command::Skip {
        reason,
        urls: Vec::new(),
    })
}

fn targeted(
    command: &FixArgs,
    config: &UserConfig,
    harness: &mut Choice,
    started_command: &mut crate::command::Started,
    reservation: Option<Reservation>,
) -> Ended {
    let launch = std::env::current_dir()
        .map(Git::new)
        .map_err(anyhow::Error::from);
    targeted_at(
        command,
        config,
        harness,
        started_command,
        reservation,
        launch,
        true,
    )
}

pub(crate) fn for_release(
    git: &Git,
    pr: &crate::review::PullRequestUrl,
    harness: &mut Choice,
) -> anyhow::Result<()> {
    let _recording = logs::nested();
    let (config, mut started) = crate::command::start(
        logs::Begin::FixRun { pass: false },
        |_| crate::notification::NotificationAsk::Skip,
        crate::notification::About::ReleaseRun,
    )
    .map_err(|_| anyhow::anyhow!("could not start targeted Fix run"))?;
    let command = FixArgs {
        target: Target::PullRequest(crate::review::PullRequestInput::Url(pr.clone())),
        flags: Flags {
            goal: Some(Goal::ReadyForReview),
            base_fix: Some(BaseFixAsk::Forbid),
            ..Default::default()
        },
    };
    let ended = targeted_at(
        &command,
        &config,
        harness,
        &mut started,
        None,
        Ok(Git::new(git.dir())),
        false,
    );
    let success = ended.outcome.is_ok();
    started.finish(crate::command::Ending::Run(ended));
    anyhow::ensure!(
        success,
        "targeted Fix run failed; review {} before retrying",
        pr.url
    );
    Ok(())
}

fn targeted_at(
    command: &FixArgs,
    config: &UserConfig,
    harness: &mut Choice,
    started_command: &mut crate::command::Started,
    reservation: Option<Reservation>,
    launch: anyhow::Result<Git>,
    occupy_slot: bool,
) -> Ended {
    let started = Utc::now() - crate::run::STARTED_MARGIN;
    let mut base_fix = None;
    let outcome = (|| -> Result<_, crate::failed_run::FailedRun> {
        let launch = launch.context("could not read Launch directory")?;
        let origin = launch.run(&["config", "remote.origin.url"])?;
        let repo = Repo::of_origin(&origin).context("fix needs a GitHub origin")?;
        let Target::PullRequest(input) = &command.target else {
            unreachable!()
        };
        let pr = input.resolve(&repo)?;
        if !pr.issue.matches_origin(&origin) {
            return Err(anyhow::anyhow!("origin mismatch: {} is not in {origin}", pr.url).into());
        }
        let github = GitHub::new();
        let snapshot = github.fix_pull_request(&pr)?;
        started_command.pull_request(&pr, &snapshot.title);
        let pass = reservation.is_some();
        let _reservation = match reservation {
            Some(held) => held,
            None => Reservation::take(&launch, &github, &pr)?
                .context("pull request is already reserved or labelled in-progress")?,
        };
        let snapshot = github.fix_pull_request(&pr)?;
        started_command.pull_request(&pr, &snapshot.title);
        if !pass && occupy_slot {
            crate::command_limit::hold_slot(&repo);
        }
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
        // Acquisition samples origin independently of PR metadata. Evidence
        // must cover the head actually adopted before Delivery begins.
        let adopted_head = worktree.head()?;
        let reviewed =
            asks.goal != Goal::Merged || github.authored_or_reviewed_head(&pr, &adopted_head)?;
        let mut target =
            PullRequest::on_pull_request(&pr, &snapshot.branch, &snapshot.base, github);
        if !reviewed {
            let reason = format!(
                "PR {} stays ready for review: adopted head {} has no recorded agent review; run thirdshift review {} before retrying Self-merge",
                pr.url, adopted_head, pr.url
            );
            // A Pass or User config merge default may hand off for review.
            // An explicit targeted merge must still report its unmet goal.
            if pass || command.flags.goal != Some(Goal::Merged) {
                crate::progress::step(format!("Self-merge held: {reason}"));
                asks.goal = Goal::ReadyForReview;
            }
            target.hold_self_merge(reason);
        }
        let result = Delivery {
            security_review: false,
            child_asks: &child_asks,
            issue: &pr.issue,
            base: &snapshot.base,
            goal: asks.goal,
            base_fix: &mut repair_base,
            logs: &logs,
            started,
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
