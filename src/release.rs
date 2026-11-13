//! Explicit-version Release runs. Source comes only from fetched Git objects;
//! the Launch directory supplies origin and object storage, never executable code.

use std::fs::{self, File};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, ensure};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::{Conclusion, GitHub, Mergeable, PrState};
use crate::harness::Choice;
use crate::issue::Repo;
use crate::{interrupt, logs, poll, progress};

mod summary;

pub const REPOSITORY: &str = "JacobStephens2/thirdshift";

/// Success requires both publication and the bump pull request's merge.
pub struct Ended {
    pub pr_url: String,
    pub version: String,
}

/// Reject factory recursion before Setup, credentials or any network access.
pub fn outside_session() -> Result<()> {
    ensure!(
        std::env::var_os("THIRDSHIFT_SESSION").is_none()
            && std::env::var_os("THIRDSHIFT_PARENT_RECORD").is_none(),
        "thirdshift release cannot run inside a factory Session"
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version([u64; 3]);

impl Version {
    fn parse(text: &str) -> Result<Self> {
        let mut parts = Vec::new();
        for part in text.split('.') {
            ensure!(
                !part.is_empty()
                    && part.bytes().all(|byte| byte.is_ascii_digit())
                    && (part.len() == 1 || !part.starts_with('0')),
                "release version must be X.Y.Z with no leading zeros: {text}"
            );
            parts.push(
                part.parse::<u64>()
                    .context("release version component is too large")?,
            );
        }
        Ok(Self(parts.try_into().map_err(|_| {
            anyhow::anyhow!("release version must be X.Y.Z: {text}")
        })?))
    }
}

/// Own the locks, worktree and live PR label through interruption cleanup.
pub fn run(version: &str, choice: &mut Choice) -> Result<Ended, FailedRun> {
    let mut pr_url = None;
    let mut trace = None;
    let result = (|| -> Result<Ended> {
        outside_session()?;
        let wanted = Version::parse(version)?;
        let launch = Git::new(std::env::current_dir()?);
        let repo = Repo::of_origin(&launch.run(&["config", "remote.origin.url"])?)
            .context("release requires a GitHub origin")?;
        ensure!(
            repo.slug().eq_ignore_ascii_case(REPOSITORY),
            "release requires origin {REPOSITORY}"
        );
        logs::started(logs::Work::ReleaseRun(&repo), choice);
        let _local = local_lock()?;
        fetch(&launch)?;
        validate_version(&launch, &wanted)?;
        let branch = format!("release-{version}");
        let tag = format!("v{version}");
        ensure!(
            launch
                .run(&[
                    "ls-remote",
                    "--refs",
                    "origin",
                    &format!("refs/heads/{branch}"),
                    &format!("refs/tags/{tag}")
                ])?
                .is_empty(),
            "{branch} or {tag} already exists on origin; recovery requires the Day shift"
        );
        ensure!(
            launch.run(&["tag", "--list", &tag])?.is_empty(),
            "{tag} already exists locally"
        );
        let _reservation = crate::reservation::Reservation::release(&launch)?
            .context("another Release run holds the origin release reservation")?;
        let github = GitHub::new();
        let (base, previous) = candidate(&launch, &github, &wanted)?;
        let work = Worktree::new(&launch, &base)?;
        let git = Git::new(work.path());
        bump(work.path(), version)?;
        git.run(&["add", "--", "Cargo.toml", "Cargo.lock"])?;
        git.run(&[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-m",
            &format!("Release {version}"),
        ])?;
        let head = git.run(&["rev-parse", "HEAD"])?;
        let diff = git.run(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            &base,
            &head,
            "--",
            "Cargo.toml",
            "Cargo.lock",
        ])?;
        let input = summary::input(&launch, &github, &base, previous.as_deref(), version, &diff)?;
        let summary = match summary::write(choice, &repo, &input, &mut trace) {
            Ok(text) => Some(text),
            Err(error) => {
                interrupt::check()?;
                progress::warn(
                    &error,
                    format_args!("release summary unavailable; using GitHub generated notes"),
                );
                None
            }
        };
        let notes = if summary.is_none() {
            github.release_notes(&tag, &base, previous.as_deref())?
        } else {
            String::new()
        };
        let body = format!(
            "## Summary\n\n<!-- release-summary:start -->\n{}\n<!-- release-summary:end -->\n\n{notes}\n\n## Version diff\n\n```diff\n{diff}\n```\n\n<!-- release-content-base: {base} -->\n",
            summary
                .as_deref()
                .unwrap_or("_The agent summary was unavailable._")
        );
        logs::stage(logs::Stage::PullRequest);
        // A release merged while the summary was running invalidates this cut.
        fetch(&launch)?;
        validate_version(&launch, &wanted)?;
        ensure!(
            last_tag(&launch)? == previous,
            "a newer release appeared while preparing this release; retry"
        );
        git.run(&[
            "push",
            "--no-verify",
            "origin",
            &format!("{head}:refs/heads/{branch}"),
        ])?;
        let url = github.open_release(version, &branch, &body)?;
        pr_url = Some(url.clone());
        let pr = crate::review::PullRequestUrl::parse(&url)?;
        logs::pull_request(&pr.url);
        let _label = LiveLabel::take(&github, &pr)?;
        logs::stage(logs::Stage::CiWatch);
        wait_for_bump(&github, &pr, &head, &branch)?;
        // Only the validated head is tagged, never a branch name that can move.
        progress::step(format_args!("publishing {tag} at {head}"));
        git.run(&[
            "push",
            "--no-verify",
            "origin",
            &format!("{head}:refs/tags/{tag}"),
        ])?;
        wait_for_release(&github, &head)?;
        check_bump(&github, &pr, &head, &branch)?;
        logs::stage(logs::Stage::SelfMerge);
        github.merge(&pr.issue, pr.issue.number, &head)
            .context("GitHub Release was published, but the bump pull request was not merged; the Day shift must merge it")?;
        ensure!(
            github.pr_state(&pr.issue, &url)? == PrState::Merged,
            "GitHub Release was published, but the bump pull request has not merged"
        );
        Ok(Ended {
            pr_url: url,
            version: version.into(),
        })
    })();
    result.map_err(|error| FailedRun {
        pr_url,
        log: trace,
        ..error.into()
    })
}

fn local_lock() -> Result<File> {
    let directory = crate::config::home()?.join(".thirdshift");
    fs::create_dir_all(&directory)?;
    let file = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("release.lock"))?;
    file.try_lock()
        .context("another Release run holds the machine-local release lock")?;
    Ok(file)
}

fn fetch(git: &Git) -> Result<()> {
    git.run(&[
        "fetch",
        "--quiet",
        "--tags",
        "origin",
        "+refs/heads/main:refs/remotes/origin/main",
    ])
    .map(drop)
}

fn validate_version(git: &Git, wanted: &Version) -> Result<()> {
    let manifest = git.run(&["show", "origin/main:Cargo.toml"])?;
    let manifest: toml::Value = toml::from_str(&manifest)?;
    let current = manifest["package"]["version"]
        .as_str()
        .context("main has no package version")?;
    ensure!(
        *wanted > Version::parse(current)?,
        "release version must be strictly greater than {current} on main"
    );
    Ok(())
}

fn last_tag(git: &Git) -> Result<Option<String>> {
    if git
        .run(&["tag", "--merged", "origin/main", "--list", "v[0-9]*"])?
        .is_empty()
    {
        return Ok(None);
    }
    git.run(&[
        "describe",
        "--tags",
        "--abbrev=0",
        "--match",
        "v[0-9]*",
        "origin/main",
    ])
    .map(Some)
}

fn candidate(git: &Git, github: &GitHub, wanted: &Version) -> Result<(String, Option<String>)> {
    let mut progress_line = poll::WaitingLine::new();
    let started = Instant::now();
    loop {
        fetch(git)?;
        validate_version(git, wanted)?;
        let previous = last_tag(git)?;
        let boundary = previous.as_ref().map(|tag| format!("^{tag}"));
        let mut args = vec!["rev-list", "--first-parent", "origin/main"];
        if let Some(boundary) = &boundary {
            args.push(boundary);
        }
        let commits = git.run(&args)?;
        let mut pending = false;
        for sha in commits.lines() {
            if let Some(previous) = &previous {
                // main can contain a release tag on a merged side branch.
                if git.run(&["merge-base", previous, sha])?
                    != git.run(&["rev-parse", &format!("{previous}^{{commit}}")])?
                {
                    continue;
                }
            }
            match github.release_push_run("ci.yml", "main", sha)? {
                Some(run)
                    if run.status == "completed"
                        && run.conclusion.as_deref() == Some("success") =>
                {
                    return Ok((sha.into(), previous));
                }
                Some(run) if run.status != "completed" => pending = true,
                None if started.elapsed() < poll::grace_period() => pending = true,
                _ => {}
            }
        }
        ensure!(
            pending,
            "no candidate main commit has successful push CI; all candidates completed non-green or have no CI"
        );
        if progress_line.should_report(()) {
            progress::step("waiting for main push CI before releasing");
        }
        poll::pause()?;
    }
}

fn check_bump(
    github: &GitHub,
    pr: &crate::review::PullRequestUrl,
    head: &str,
    branch: &str,
) -> Result<Mergeable> {
    let snapshot = github
        .pr_snapshot(&pr.issue, &pr.url)?
        .context("release bump pull request disappeared")?;
    ensure!(
        snapshot.pr.state == PrState::Open
            && !snapshot.pr.is_draft
            && !snapshot.from_fork
            && snapshot.pr.head == branch
            && snapshot.pr.base == "main"
            && snapshot.head_commit == head,
        "release bump pull request is no longer open and ready at the validated head; the Day shift must review it"
    );
    ensure!(
        snapshot.mergeable != Mergeable::No,
        "release bump pull request has a merge conflict"
    );
    Ok(snapshot.mergeable)
}

fn wait_for_bump(
    github: &GitHub,
    pr: &crate::review::PullRequestUrl,
    head: &str,
    branch: &str,
) -> Result<()> {
    let mut line = poll::WaitingLine::new();
    let started = Instant::now();
    let mut green_since = None;
    loop {
        let mergeable = check_bump(github, pr, head, branch)?;
        let checks = github.checks_on(&pr.issue, head)?;
        for check in &checks {
            ensure!(
                matches!(
                    check.conclusion,
                    Conclusion::Success
                        | Conclusion::Skipped
                        | Conclusion::Neutral
                        | Conclusion::Pending
                        | Conclusion::Unfinished
                ),
                "release bump check {} failed: {:?}",
                check.name,
                check.conclusion
            );
        }
        let green = (!checks.is_empty() || started.elapsed() >= poll::grace_period())
            && checks.iter().all(|check| {
                matches!(
                    check.conclusion,
                    Conclusion::Success | Conclusion::Skipped | Conclusion::Neutral
                )
            });
        if green {
            if mergeable == Mergeable::Yes {
                return Ok(());
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

fn wait_for_release(github: &GitHub, head: &str) -> Result<()> {
    let mut line = poll::WaitingLine::new();
    let started = Instant::now();
    loop {
        let run = github.release_push_run("release.yml", "", head)?;
        if let Some(run) = &run {
            if run.status == "completed" {
                ensure!(
                    run.conclusion.as_deref() == Some("success"),
                    "release workflow failed: {:?}; tag is left for the Day shift",
                    run.conclusion
                );
                return Ok(());
            }
        } else {
            ensure!(
                started.elapsed() < poll::grace_period(),
                "release workflow did not appear; tag is left for the Day shift"
            );
        }
        if line.should_report(()) {
            progress::step("waiting for release workflow publication");
        }
        poll::pause()?;
    }
}

fn bump(directory: &Path, version: &str) -> Result<()> {
    let manifest_path = directory.join("Cargo.toml");
    let mut manifest: toml_edit::DocumentMut = fs::read_to_string(&manifest_path)?.parse()?;
    let name = manifest["package"]["name"]
        .as_str()
        .context("manifest has no package name")?
        .to_owned();
    let old = manifest["package"]["version"]
        .as_str()
        .context("manifest has no version")?
        .to_owned();
    manifest["package"]["version"] = toml_edit::value(version);
    let lock_path = directory.join("Cargo.lock");
    let mut lock: toml_edit::DocumentMut = fs::read_to_string(&lock_path)?.parse()?;
    let mut found = 0;
    for package in lock["package"]
        .as_array_of_tables_mut()
        .context("lockfile has no packages")?
        .iter_mut()
    {
        if package["name"].as_str() == Some(&name)
            && package["version"].as_str() == Some(&old)
            && !package.contains_key("source")
        {
            package["version"] = toml_edit::value(version);
            found += 1;
        }
    }
    ensure!(
        found == 1,
        "lockfile must contain exactly one local package {name} at {old}"
    );
    fs::write(manifest_path, manifest.to_string())?;
    fs::write(lock_path, lock.to_string())?;
    Ok(())
}

struct Worktree<'a> {
    launch: &'a Git,
    directory: tempfile::TempDir,
}
impl<'a> Worktree<'a> {
    fn new(launch: &'a Git, base: &str) -> Result<Self> {
        let directory = tempfile::Builder::new()
            .prefix("thirdshift-release-")
            .tempdir()?;
        let held = Self { launch, directory };
        launch.run(&[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            held.path().to_str().context("non-UTF8 release directory")?,
            base,
        ])?;
        Ok(held)
    }
    fn path(&self) -> &Path {
        self.directory.path()
    }
}
impl Drop for Worktree<'_> {
    fn drop(&mut self) {
        if let Err(error) = self.launch.completion().run(&[
            "worktree",
            "remove",
            "--force",
            &self.path().to_string_lossy(),
        ]) {
            progress::warn(&error, format_args!("could not remove release worktree"));
        }
    }
}

struct LiveLabel {
    pr: crate::review::PullRequestUrl,
}
impl LiveLabel {
    fn take(github: &GitHub, pr: &crate::review::PullRequestUrl) -> Result<Self> {
        let held = Self { pr: pr.clone() };
        let labels = github.review_labels(pr)?;
        crate::labels::Edit::of(&pr.issue, labels, &[], &[crate::claim::IN_PROGRESS])
            .apply(github)?;
        Ok(held)
    }
}
impl Drop for LiveLabel {
    fn drop(&mut self) {
        if let Err(error) = GitHub::new()
            .completion()
            .remove_label(&self.pr.issue, crate::claim::IN_PROGRESS.name())
        {
            progress::warn(
                &error,
                format_args!("could not remove release in-progress label"),
            );
        }
    }
}
