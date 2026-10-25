use std::collections::BTreeSet;

use anyhow::{Context, Result, bail, ensure};

use super::{Proposal, PullRequest, Settlement, State};
use crate::config::UserConfig;
use crate::github::GitHub;
use crate::issue::IssueUrl;
use crate::security::fixing::SECURITY_FIX;
use crate::{config, evals, scoring};

enum Action {
    Confirm,
    Correct(String),
    Reject,
}

pub(crate) struct Options {
    issue: Option<String>,
    action: Option<Action>,
    json: bool,
}

pub(crate) fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options {
        issue: None,
        action: None,
        json: false,
    };
    let mut arguments = args.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--none" if options.action.is_none() => options.action = Some(Action::Reject),
            "--confirm" if options.action.is_none() => options.action = Some(Action::Confirm),
            "--correct" if options.action.is_none() => {
                options.action = Some(Action::Correct(
                    arguments
                        .next()
                        .context("--correct requires a pull request")?
                        .clone(),
                ));
            }
            "--json" if !options.json => options.json = true,
            value if !value.starts_with('-') && options.issue.is_none() => {
                options.issue = Some(value.into())
            }
            _ => bail!(
                "usage: thirdshift bug-origins [<issue>] [--confirm | --correct <pull request> | --none] [--json]"
            ),
        }
    }
    ensure!(
        options.action.is_none() || options.issue.is_some(),
        "settling a Bug origin requires an issue"
    );
    Ok(options)
}

pub(crate) fn execute(options: Options) -> Result<String> {
    if options.action.is_some() {
        crate::interrupt::install()?;
    }
    apply(options)
}

/// Shared operation; process signal handling belongs to the CLI entry point.
pub(crate) fn apply(options: Options) -> Result<String> {
    let _lock = if options.action.is_some() {
        Some(scoring::lock()?.context("another Scoring writer is running")?)
    } else {
        None
    };
    let directory = scoring::directory()?;
    let mut weeks = scoring::read(&directory)?;
    let issue = options
        .issue
        .as_deref()
        .map(|target| {
            let matches: BTreeSet<_> = weeks
                .iter()
                .flat_map(|week| week.bug_origins.iter().flatten())
                .filter(|origin| {
                    origin
                        .issue
                        .trim_end_matches('/')
                        .eq_ignore_ascii_case(target.trim_end_matches('/'))
                        || origin.issue.rsplit('/').next() == Some(target)
                })
                .map(|origin| origin.issue.clone())
                .collect();
            ensure!(
                matches.len() == 1,
                "Bug origin is unknown or ambiguous: {target}"
            );
            Ok::<_, anyhow::Error>(matches.into_iter().next().unwrap())
        })
        .transpose()?;
    if let Some(action) = options.action {
        let previous: Vec<_> = weeks
            .iter()
            .flat_map(|week| week.bug_origins.iter().flatten())
            .filter(|origin| Some(&origin.issue) == issue.as_ref())
            .filter_map(|origin| {
                origin
                    .settlement
                    .as_ref()
                    .map(|settlement| settlement.eval_task.clone())
            })
            .collect();
        let week = weeks
            .iter_mut()
            .rev()
            .find(|week| {
                week.bug_origins.as_ref().is_some_and(|origins| {
                    origins
                        .iter()
                        .any(|origin| Some(&origin.issue) == issue.as_ref())
                })
            })
            .unwrap();
        let origin = week
            .bug_origins
            .as_mut()
            .unwrap()
            .iter_mut()
            .find(|origin| Some(&origin.issue) == issue.as_ref())
            .unwrap();
        settle(origin, action)?;
        let keep = origin
            .settlement
            .as_ref()
            .map(|settlement| settlement.eval_task.clone());
        config::replace(
            &scoring::checkpoint(&directory, week)?,
            &serde_json::to_string_pretty(week)?,
        )?;
        // Reopened and reclosed bugs can have several retained occurrences.
        // The latest decision supersedes all their pending proposals.
        for reference in previous {
            if keep
                .as_ref()
                .is_none_or(|keep| keep.suite != reference.suite || keep.task != reference.task)
            {
                evals::bug_origin::withdraw(&reference)?;
            }
        }
    }
    let origins: Vec<Proposal> = listed(weeks)
        .into_iter()
        .filter(|origin| issue.as_ref().is_none_or(|issue| &origin.issue == issue))
        .collect();
    if options.json {
        return Ok(serde_json::to_string_pretty(&origins)?);
    }
    let mut text = String::new();
    for origin in origins {
        let state = origin.state.name();
        text.push_str(&format!("{}  {state}\n{}\n", origin.issue, origin.reason));
        if let Some(settlement) = &origin.settlement {
            text.push_str(&format!(
                "{}{}\nEval task: {} / {}\n",
                settlement.pull_request.url,
                if settlement.corrected {
                    " (corrected)"
                } else {
                    ""
                },
                settlement.eval_task.suite,
                settlement.eval_task.task
            ));
        } else {
            for pr in &origin.pull_requests {
                text.push_str(&format!("{}\n", pr.url));
            }
        }
    }
    Ok(text)
}

pub(crate) fn view() -> Result<Vec<Proposal>> {
    Ok(listed(scoring::read(&scoring::directory()?)?))
}

fn listed(weeks: Vec<scoring::Week>) -> Vec<Proposal> {
    weeks
        .into_iter()
        .flat_map(|week| week.bug_origins.unwrap_or_default())
        .collect()
}

fn settle(origin: &mut Proposal, action: Action) -> Result<()> {
    if matches!(action, Action::Reject) {
        if let Some(settlement) = &origin.settlement {
            evals::bug_origin::withdraw(&settlement.eval_task)?;
        }
        origin.state = State::Rejected;
        origin.settlement = None;
        return Ok(());
    }
    let bug = IssueUrl::parse(&origin.issue)?;
    let (url, corrected) = match action {
        Action::Confirm if origin.state == State::Confirmed => return Ok(()),
        Action::Confirm => {
            ensure!(
                origin.pull_requests.len() == 1,
                "Bug origin has no unique proposed pull request; use --correct <pull request>"
            );
            (origin.pull_requests[0].url.clone(), false)
        }
        Action::Correct(value) => {
            let url = if value.bytes().all(|byte| byte.is_ascii_digit()) && !value.is_empty() {
                format!("https://github.com/{}/pull/{value}", bug.repo_slug())
            } else {
                value
            };
            (url, true)
        }
        Action::Reject => unreachable!(),
    };
    let pr = IssueUrl::parse(&url.replace("/pull/", "/issues/"))?;
    ensure!(
        url.contains("/pull/") && pr.number > 0 && pr.in_same_repo(&bug),
        "Bug-origin pull request must belong to the bug's repository"
    );
    let url = format!("https://github.com/{}/pull/{}", pr.repo_slug(), pr.number);
    if origin.state == State::Confirmed
        && origin
            .settlement
            .as_ref()
            .is_some_and(|settlement| settlement.pull_request.url == url)
    {
        return Ok(());
    }
    let github = GitHub::new();
    // A proposal saved before the label still must not become an Eval task.
    ensure!(
        !github.issue_labels(&bug)?.has(SECURITY_FIX),
        "Security fixes cannot become Bug-origin Eval tasks"
    );
    let review = github.bug_origin_review(&pr)?;
    ensure!(
        review.merged_at <= origin.closed_at,
        "Bug-origin pull request merged after the bug was closed"
    );
    let pull_request = PullRequest::attributed(&UserConfig::load()?.logs_dir, url, &review.body)?;
    let eval_task = evals::bug_origin::propose(&bug, &pr, review, &origin.reason)?;
    if let Some(previous) = &origin.settlement {
        evals::bug_origin::withdraw(&previous.eval_task)?;
    }
    origin.settlement = Some(Settlement {
        pull_request,
        corrected,
        eval_task,
    });
    origin.state = State::Confirmed;
    Ok(())
}
