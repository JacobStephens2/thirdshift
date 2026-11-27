//! Pass admission and issue selection, before existing-issue triage starts.

use anyhow::Result;

use super::{Input, Options, existing, investigate};
use crate::asks::Flags;
use crate::command::Skip;
use crate::command_limit::Sharing;
use crate::config::UserConfig;
use crate::failed_run::FailedRun;
use crate::github::{GitHub, ListedIssue};
use crate::issue::Repo;
use crate::labels;
use crate::launch::{self, Launch, Start};
use crate::logs::{self, Pass, Work};
use crate::pass::LaunchAndGitHub;
use crate::ready::PickupOrder;
use crate::reservation::Reservation;

pub enum Outcome {
    Ran(super::Ended),
    Skipped(Skip),
}

fn skipped(repo: &Repo, reason: impl std::fmt::Display) -> Outcome {
    logs::skipped(Pass::Triage, repo, &reason);
    Outcome::Skipped(Skip {
        reason: reason.to_string(),
        urls: Vec::new(),
    })
}

pub fn run(
    base: Option<&str>,
    options: Options,
    flags: &Flags,
    config: &UserConfig,
    harness: &mut crate::harness::Choice,
    skip_authors: &crate::skip_authors::SkipAuthors,
) -> Result<Outcome, FailedRun> {
    let Launch {
        directory,
        repo,
        base,
    } = match launch::start(base, config.command_limit, Sharing::UpToTheLimit, harness)? {
        Start::Clear(launch) => launch,
        Start::NotAdmitted(running) => return Ok(skipped(&running.repo, &running)),
    };
    let failed = crate::pickup::failures(&mut LaunchAndGitHub {
        skip_authors: &Default::default(),
        launch: directory.git(),
        base: &base,
        repo: &repo,
        harness,
        flags,
        config,
        security_review_harness: None,
    })?;
    if failed.len() >= config.pickup_failure_limit.get() {
        return Ok(skipped(
            &repo,
            crate::pickup::Skipped::AtFailureLimit {
                repo: repo.clone(),
                failed,
                limit: config.pickup_failure_limit,
            },
        ));
    }
    let github = GitHub::new();
    let listed = github.open_candidate_issues(&repo.slug(), labels::NEEDS_TRIAGE)?;
    logs::listing_disagreement(&repo, &listed);
    let mut candidates = listed.issues;
    candidates.sort_by_key(|listed| listed.issue.number);
    if config.pickup_order == PickupOrder::HighestFirst {
        candidates.reverse();
    }
    for listed in candidates {
        let issue = &listed.issue;
        if let Some(reason) = ineligible(&listed, &github, skip_authors)? {
            crate::progress::step(format!("#{} {reason}", issue.number));
            continue;
        }
        let Some(origin) = Reservation::take(directory.git(), issue)? else {
            crate::progress::step(format!(
                "#{} is reserved by another run on origin",
                issue.number
            ));
            continue;
        };
        let current = github.issue(issue)?;
        let title = current.title.or_else(|| Some(listed.title.clone()));
        let reason = if current.is_open {
            ineligible(
                &ListedIssue {
                    issue: issue.clone(),
                    title: listed.title,
                    labels: current.labels,
                },
                &github,
                skip_authors,
            )?
        } else {
            Some("is closed".into())
        };
        if let Some(reason) = reason {
            crate::progress::step(format!("#{} selection went stale: {reason}", issue.number));
            continue;
        }
        crate::progress::step(format!("triaging {}", issue.url));
        logs::started(
            Work::TriageRun {
                repo: &repo,
                issue: Some(issue),
            },
            harness,
        );
        logs::issue_title(title);
        base.pull();
        let mut origin = Some(origin);
        let claim = if options.recommend_only {
            None
        } else {
            Some(existing::Claim::reserved(
                issue,
                origin.take().expect("held reservation"),
            )?)
        };
        let input = Input::Issue(issue.clone());
        return investigate(
            &input,
            options,
            harness,
            &repo,
            directory.git(),
            &base,
            claim,
        )
        .map(Outcome::Ran);
    }
    Ok(skipped(
        &repo,
        format!(
            "no eligible needs-triage issue on {} it can reserve",
            repo.slug()
        ),
    ))
}

fn ineligible(
    listed: &ListedIssue,
    github: &GitHub,
    skip_authors: &crate::skip_authors::SkipAuthors,
) -> Result<Option<String>> {
    if !listed.labels.has(labels::NEEDS_TRIAGE) {
        return Ok(Some("is no longer labelled needs-triage".into()));
    }
    if let Some(label) = listed.labels.triage_disqualifiers().into_iter().next() {
        return Ok(Some(format!("is labelled {label}")));
    }
    let candidate = github.candidate(&listed.issue, labels::NEEDS_TRIAGE)?;
    if candidate.claimed {
        return Ok(Some("is already Claimed".into()));
    }
    if candidate.has_sub_issues() {
        return Ok(Some("is a parent Spec with sub-issues".into()));
    }
    if let Some(reason) = existing::trust_reason(&listed.issue, github)? {
        return Ok(Some(reason));
    }
    Ok(skip_authors
        .reason(github, &listed.issue)?
        .map(|reason| format!("passed over: {reason}")))
}
