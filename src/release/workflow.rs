//! Publication ordering and one targeted Fix attempt for a captured bump PR.
use super::*;

pub(super) struct Notes<'a> {
    pub repo: &'a Repo,
    pub choice: &'a mut Choice,
    pub trace: &'a mut Option<std::path::PathBuf>,
}

pub(super) fn finish(
    git: &Git,
    github: &GitHub,
    cut: &mut recovery::Cut,
    request: &Request<'_>,
    notes: &mut Notes<'_>,
    automatic: Option<Option<String>>,
    pr_url: &mut Option<String>,
) -> Result<()> {
    let mut label = None;
    if !cut.published {
        let same_tag = cut.tagged.as_deref() == Some(&cut.head);
        if cut.tagged.is_none() || cut.recut || !same_tag {
            let merged = match &cut.pr {
                Some(pr) => github.pr_state(&pr.issue, &pr.url)? == PrState::Merged,
                None => false,
            };
            controls::experiment(git, cut, request.with_experiment)?;
            let body = notes.body(git, github, cut, request.review, automatic)?;
            fetch(git)?;
            recovery::ensure_previous(git, github, cut.previous.as_deref())?;
            if !((merged || cut.recut && cut.tagged.is_some())
                && Version::parse(&cut.version)? == main_version(git)?)
            {
                validate_version(git, &Version::parse(&cut.version)?)?;
            }
            cut.push(git, github)?;
            if let Some(pr) = &cut.pr {
                github.set_pr_body(&pr.issue, pr.issue.number, &body)?;
            } else {
                cut.pr = Some(crate::review::PullRequestUrl::parse(
                    &github.open_release(&cut.version, &cut.branch(), &body)?,
                )?);
            }
        }
        let pr = cut
            .pr
            .as_ref()
            .context("release tag has no bump pull request; restore or open its exact-bump PR")?;
        *pr_url = Some(pr.url.clone());
        logs::pull_request(&pr.url);
        let merged = github.pr_state(&pr.issue, &pr.url)? == PrState::Merged;
        if !same_tag || cut.tagged.is_none() {
            if !merged {
                label = Some(LiveLabel::take(github, pr)?);
                ready(git, github, cut, request, notes, &mut label)?;
            }
            ensure!(
                !github.release_published(&cut.tag())?,
                "{} was published while waiting; its tag will not be moved",
                cut.tag()
            );
            let tag_ref = format!("refs/tags/{}", cut.tag());
            recovery::ensure_previous(git, github, cut.previous.as_deref())?;
            let old = cut.expected_tag.as_deref().unwrap_or("");
            let lease = format!("--force-with-lease={tag_ref}:{old}");
            progress::step(format_args!("publishing {} at {}", cut.tag(), cut.head));
            git_run(
                git,
                &[
                    "push",
                    "--no-verify",
                    &lease,
                    "origin",
                    &format!("{}:{tag_ref}", cut.head),
                ],
            )?;
        } else if let Some(run) = github.release_push_run("release.yml", "", &cut.head)?
            && run.status == "completed"
            && run.conclusion.as_deref() != Some("success")
        {
            github.rerun_failed_jobs(&pr.issue, run.id)?;
            let mut line = poll::WaitingLine::new();
            loop {
                let next = github
                    .release_push_run("release.yml", "", &cut.head)?
                    .context("release workflow disappeared after rerun")?;
                if next.attempt != run.attempt
                    || next.status != run.status
                    || next.conclusion != run.conclusion
                {
                    break;
                }
                if line.should_report(()) {
                    progress::step("waiting for failed release workflow rerun to start");
                }
                poll::pause()?;
            }
        }
        wait_for_release(github, &cut.head)?;
        ensure!(
            github.release_published(&cut.tag())?,
            "release workflow succeeded but {} is not published; inspect GitHub Releases before retrying",
            cut.tag()
        );
    }
    let pr = cut
        .pr
        .as_ref()
        .context("published release has no bump PR; the Day shift must restore it")?;
    *pr_url = Some(pr.url.clone());
    logs::pull_request(&pr.url);
    if github.pr_state(&pr.issue, &pr.url)? == PrState::Merged {
        if let Some(old) = &cut.supersedes {
            github.close_superseded_release(old)?;
        }
        return Ok(());
    }
    check_bump(github, pr, &cut.head, &cut.branch())?;
    logs::stage(logs::Stage::SelfMerge);
    ensure!(
        !github.review_blocks_merge(&pr.issue, pr.issue.number),
        "review required before merging the release bump pull request; the GitHub Release remains published"
    );
    if let Err(error) = github.merge(&pr.issue, pr.issue.number, &cut.head) {
        // A PR turned draft after validation (the 0.29.0 recovery) needs
        // marking ready before anyone can merge it.
        let draft = matches!(
            github.pr_snapshot(&pr.issue, &pr.url),
            Ok(Some(snapshot)) if snapshot.pr.is_draft
        );
        let step = if draft {
            format!(
                "{} is a draft; review it and run gh pr ready {}, then merge it",
                pr.url, pr.url
            )
        } else {
            format!("the Day shift must merge {}", pr.url)
        };
        return Err(error.context(format!(
            "GitHub Release was published, but the bump pull request was not merged; {step}"
        )));
    }
    ensure!(
        github.pr_state(&pr.issue, &pr.url)? == PrState::Merged,
        "GitHub Release was published, but the bump pull request has not merged"
    );
    drop(label);
    if let Some(old) = &cut.supersedes {
        github.close_superseded_release(old)?;
    }
    Ok(())
}

impl Notes<'_> {
    fn body(
        &mut self,
        git: &Git,
        github: &GitHub,
        cut: &recovery::Cut,
        review: bool,
        automatic: Option<Option<String>>,
    ) -> Result<String> {
        let repo = self.repo;
        let choice = &mut *self.choice;
        let trace = &mut *self.trace;
        let diff = git_run(
            git,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                &cut.base,
                &cut.head,
                "--",
                "Cargo.toml",
                "Cargo.lock",
            ],
        )?;
        let summary = if let Some(summary) = automatic {
            summary
        } else {
            let input = summary::input(
                git,
                github,
                &cut.base,
                cut.previous.as_deref(),
                &cut.version,
                &diff,
            )?;
            match summary::write(choice, repo, &input, trace) {
                Ok(summary) => Some(summary),
                Err(error) => {
                    interrupt::check()?;
                    progress::warn(
                        &error,
                        format_args!("release summary unavailable; using GitHub generated notes"),
                    );
                    None
                }
            }
        };
        let notes = if summary.is_none() {
            github.release_notes(&cut.tag(), &cut.base, cut.previous.as_deref())?
        } else {
            String::new()
        };
        let summary = controls::review(
            summary.unwrap_or_else(|| "_The agent summary was unavailable._".into()),
            review,
        )?;
        Ok(format!(
            "## Summary\n\n<!-- release-summary:start -->\n{summary}\n<!-- release-summary:end -->\n\n{notes}\n\n## Version diff\n\n```diff\n{diff}\n```\n\n<!-- release-content-base: {} -->\n",
            cut.base
        ))
    }
}

fn ready(
    git: &Git,
    github: &GitHub,
    cut: &mut recovery::Cut,
    request: &Request<'_>,
    notes: &mut Notes<'_>,
    label: &mut Option<LiveLabel>,
) -> Result<()> {
    logs::stage(logs::Stage::CiWatch);
    let mut fixed = false;
    let mut changes = 0;
    let mut started = Instant::now();
    let mut green_since = None;
    let mut line = poll::WaitingLine::new();
    let mut measured_base = cut.base.clone();
    loop {
        let pr = cut.pr.as_ref().context("release has no PR")?.clone();
        let snapshot = github
            .pr_snapshot(&pr.issue, &pr.url)?
            .context("release PR disappeared")?;
        bump_identity(&snapshot, &cut.branch())?;
        ensure!(
            snapshot.pr.state == PrState::Open,
            "{} is closed, draft, or changed identity; review it (gh pr ready {} for a draft), then thirdshift release {}",
            pr.url,
            pr.url,
            cut.version
        );
        if snapshot.head_commit != cut.head {
            ensure!(
                changes < 5,
                "release head keeps changing; inspect {} before retrying",
                pr.url
            );
            changes += 1;
            cut.head = snapshot.head_commit;
            started = Instant::now();
            green_since = None;
        }
        cut.validate(git, github)?;
        if cut.base != measured_base {
            let refreshed_base = cut.base.clone();
            controls::experiment(git, cut, request.with_experiment)?;
            let body = notes.body(git, github, cut, request.review, None)?;
            cut.validate(git, github)?;
            if cut.base != refreshed_base {
                continue;
            }
            github.set_pr_body(&pr.issue, pr.issue.number, &body)?;
            measured_base = refreshed_base;
            continue;
        }
        let checks = github.checks_on(&pr.issue, &cut.head)?;
        let failed = checks.iter().any(|check| {
            !matches!(
                check.conclusion,
                Conclusion::Success
                    | Conclusion::Skipped
                    | Conclusion::Neutral
                    | Conclusion::Pending
                    | Conclusion::Unfinished
            )
        });
        if snapshot.mergeable == Mergeable::No || failed {
            ensure!(
                !fixed,
                "{} remains failing/conflicted after one targeted Fix run; recover with thirdshift fix {} --no-merge --no-base-fix, then thirdshift release {}",
                pr.url,
                pr.url,
                cut.version
            );
            fixed = true;
            drop(label.take());
            progress::step(format_args!(
                "starting one targeted Fix run: thirdshift fix {} --no-merge --no-base-fix",
                pr.url
            ));
            crate::fix::for_release(git, &pr, notes.choice)?;
            *label = Some(LiveLabel::take(github, &pr)?);
            continue;
        }
        let green = !checks.is_empty()
            && checks.iter().all(|check| {
                matches!(
                    check.conclusion,
                    Conclusion::Success | Conclusion::Skipped | Conclusion::Neutral
                )
            });
        ensure!(
            !checks.is_empty() || started.elapsed() < poll::grace_period(),
            "no release bump checks appeared at {}; inspect GitHub Actions, then thirdshift release {}",
            cut.head,
            cut.version
        );
        if green {
            if snapshot.mergeable == Mergeable::Yes {
                cut.validate(git, github)?;
                if cut.base == measured_base {
                    return Ok(());
                }
                continue;
            }
            let since = green_since.get_or_insert_with(Instant::now);
            ensure!(
                since.elapsed() < poll::grace_period(),
                "GitHub did not report the green bump pull request mergeable"
            );
        }
        if line.should_report(()) {
            progress::step("waiting for release bump pull-request checks and mergeability");
        }
        poll::pause()?;
    }
}
