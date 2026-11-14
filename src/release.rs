//! Release runs and scheduled Passes. Source comes only from fetched Git objects;
//! the Launch directory supplies origin and object storage, never executable code.

use std::fs::{self, File, TryLockError};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, ensure};

use crate::failed_run::FailedRun;
use crate::git::Git;
use crate::github::{Conclusion, GitHub, Mergeable, PrState};
use crate::harness::Choice;
use crate::issue::Repo;
use crate::{interrupt, logs, poll, progress};

mod selection;
mod summary;

pub const REPOSITORY: &str = "JacobStephens2/thirdshift";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

impl Bump {
    pub fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "--major" => Some(Self::Major),
            "--minor" => Some(Self::Minor),
            "--patch" => Some(Self::Patch),
            _ => None,
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            Self::Major => "--major",
            Self::Minor => "--minor",
            Self::Patch => "--patch",
        }
    }
}

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
    fn bumped(&self, bump: Bump) -> Result<String> {
        let [major, minor, patch] = self.0;
        let next = |value: u64| value.checked_add(1).context("release version overflow");
        Ok(match bump {
            Bump::Major => format!("{}.0.0", next(major)?),
            Bump::Minor => format!("{major}.{}.0", next(minor)?),
            Bump::Patch => format!("{major}.{minor}.{}", next(patch)?),
        })
    }

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
pub fn run(
    version: Option<&str>,
    pinned: Option<Bump>,
    choice: &mut Choice,
) -> Result<Option<Ended>, FailedRun> {
    let mut pr_url = None;
    let mut trace = None;
    let result = (|| -> Result<Option<Ended>> {
        outside_session()?;
        let explicit = version.map(Version::parse).transpose()?;
        let pass = version.is_none();
        let launch = Git::new(std::env::current_dir()?);
        let repo = Repo::of_origin(&git_run(&launch, &["config", "remote.origin.url"])?)
            .context("release requires a GitHub origin")?;
        ensure!(
            repo.slug().eq_ignore_ascii_case(REPOSITORY),
            "release requires origin {REPOSITORY}"
        );
        if pass {
            logs::pass_fired(&repo);
            if let Some(reason) =
                crate::command_limit::early_skip(&repo, std::num::NonZeroUsize::MIN)
            {
                return skipped(&repo, reason);
            }
        } else {
            logs::started(logs::Work::ReleaseRun(&repo), choice);
        }
        let Some(_local) = local_lock()? else {
            if pass {
                return skipped(
                    &repo,
                    "another Release run holds the machine-local release lock",
                );
            }
            anyhow::bail!("another Release run holds the machine-local release lock");
        };
        // Declared before the reservation so Drop pushes its delete first.
        let isolated = Isolated::open(&launch)?;
        let git = &isolated.git;
        fetch(git)?;
        let _reservation = match crate::reservation::Reservation::release(git)? {
            Some(reservation) => reservation,
            None if pass => {
                return skipped(
                    &repo,
                    "another Release run holds the origin release reservation",
                );
            }
            None => anyhow::bail!("another Release run holds the origin release reservation"),
        };
        let github = GitHub::new();
        let (base, previous) = match selection::candidate(git, &github, explicit.as_ref())? {
            selection::Candidate::Ready { base, previous } => (base, previous),
            selection::Candidate::Skipped(reason) => return skipped(&repo, reason),
        };
        if pass {
            logs::started(logs::Work::ReleaseRun(&repo), choice);
        }
        let mut automatic_summary = None;
        let version = if let Some(version) = version {
            version.to_owned()
        } else {
            let current = main_version(git)?;
            if let Some(bump) = pinned {
                current.bumped(bump)?
            } else {
                let minor = current.bumped(Bump::Minor)?;
                let patch = current.bumped(Bump::Patch)?;
                let input = summary::selection_input(
                    git,
                    &github,
                    &base,
                    previous.as_deref(),
                    &minor,
                    &patch,
                )?;
                let selected = summary::select(choice, &repo, &input, &mut trace)?;
                automatic_summary = Some(selected.summary);
                if selected.bump == Bump::Minor {
                    minor
                } else {
                    patch
                }
            }
        };
        let version = version.as_str();
        let wanted = Version::parse(version)?;
        validate_version(git, &wanted)?;
        let branch = format!("release-{version}");
        let tag = format!("v{version}");
        ensure!(
            git_run(
                git,
                &[
                    "ls-remote",
                    "--refs",
                    "origin",
                    &format!("refs/heads/{branch}"),
                    &format!("refs/tags/{tag}")
                ]
            )?
            .is_empty(),
            "{branch} or {tag} already exists on origin; recovery requires the Day shift"
        );
        ensure!(
            git_run(&launch, &["tag", "--list", &tag])?.is_empty(),
            "{tag} already exists locally"
        );
        checkout(git, &base)?;
        bump(git.dir(), version)?;
        git_run(git, &["add", "--", "Cargo.toml", "Cargo.lock"])?;
        git_run(
            git,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "-m",
                &format!("Release {version}"),
            ],
        )?;
        let head = git_run(git, &["rev-parse", "HEAD"])?;
        let diff = git_run(
            git,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                &base,
                &head,
                "--",
                "Cargo.toml",
                "Cargo.lock",
            ],
        )?;
        let summary = if let Some(summary) = automatic_summary {
            summary
        } else {
            let input = summary::input(git, &github, &base, previous.as_deref(), version, &diff)?;
            match summary::write(choice, &repo, &input, &mut trace) {
                Ok(text) => Some(text),
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
        fetch(git)?;
        validate_version(git, &wanted)?;
        ensure!(
            last_tag(git)? == previous,
            "a newer release appeared while preparing this release; retry"
        );
        git_run(
            git,
            &[
                "push",
                "--no-verify",
                "origin",
                &format!("{head}:refs/heads/{branch}"),
            ],
        )?;
        let url = github.open_release(version, &branch, &body)?;
        pr_url = Some(url.clone());
        let pr = crate::review::PullRequestUrl::parse(&url)?;
        logs::pull_request(&pr.url);
        let _label = LiveLabel::take(&github, &pr)?;
        logs::stage(logs::Stage::CiWatch);
        wait_for_bump(&github, &pr, &head, &branch)?;
        // Only the validated head is tagged, never a branch name that can move.
        progress::step(format_args!("publishing {tag} at {head}"));
        git_run(
            git,
            &[
                "push",
                "--no-verify",
                "origin",
                &format!("{head}:refs/tags/{tag}"),
            ],
        )?;
        wait_for_release(&github, &head)?;
        check_bump(&github, &pr, &head, &branch)?;
        logs::stage(logs::Stage::SelfMerge);
        ensure!(
            !github.review_blocks_merge(&pr.issue, pr.issue.number),
            "review required before merging the release bump pull request; the GitHub Release remains published"
        );
        github.merge(&pr.issue, pr.issue.number, &head)
            .context("GitHub Release was published, but the bump pull request was not merged; the Day shift must merge it")?;
        ensure!(
            github.pr_state(&pr.issue, &url)? == PrState::Merged,
            "GitHub Release was published, but the bump pull request has not merged"
        );
        Ok(Some(Ended {
            pr_url: url,
            version: version.into(),
        }))
    })();
    result.map_err(|error| FailedRun {
        pr_url,
        log: trace,
        ..error.into()
    })
}

fn skipped(repo: &Repo, reason: impl std::fmt::Display) -> Result<Option<Ended>> {
    logs::skipped(logs::Pass::Release, repo, reason);
    Ok(None)
}

fn local_lock() -> Result<Option<File>> {
    let directory = crate::config::home()?.join(".thirdshift");
    fs::create_dir_all(&directory)?;
    let file = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("release.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn fetch(git: &Git) -> Result<()> {
    git_run(
        git,
        &[
            "fetch",
            "--quiet",
            "--tags",
            "origin",
            "+refs/heads/main:refs/remotes/origin/main",
        ],
    )
    .map(drop)
}

fn validate_version(git: &Git, wanted: &Version) -> Result<()> {
    let current = main_version(git)?;
    ensure!(
        *wanted > current,
        "release version must be strictly greater than {}.{}.{} on main",
        current.0[0],
        current.0[1],
        current.0[2]
    );
    Ok(())
}

fn main_version(git: &Git) -> Result<Version> {
    let manifest = git_run(git, &["show", "origin/main:Cargo.toml"])?;
    let manifest: toml::Value = toml::from_str(&manifest)?;
    let current = manifest["package"]["version"]
        .as_str()
        .context("main has no package version")?;
    Version::parse(current)
}

fn last_tag(git: &Git) -> Result<Option<String>> {
    if git_run(
        git,
        &["tag", "--merged", "origin/main", "--list", "v[0-9]*"],
    )?
    .is_empty()
    {
        return Ok(None);
    }
    git_run(
        git,
        &[
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*",
            "origin/main",
        ],
    )
    .map(Some)
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

/// Git commands for the release run in a repository this process created.
/// The Launch directory's hooks, filters, fsmonitor, attributes and remote
/// helpers never apply. Its effective origin is copied only when that URL is
/// the configured GitHub repository or a local path.
struct Isolated {
    _directory: tempfile::TempDir,
    git: Git,
}

impl Isolated {
    fn open(launch: &Git) -> Result<Self> {
        let origin = git_run(launch, &["remote", "get-url", "origin"])?;
        ensure!(
            usable_origin(&origin),
            "release origin URL is not {REPOSITORY} or a local path"
        );
        let directory = tempfile::Builder::new()
            .prefix("thirdshift-release-")
            .tempdir()?;
        let template = tempfile::Builder::new()
            .prefix("thirdshift-release-template-")
            .tempdir()?;
        let git = Git::new(directory.path());
        git_run(
            &git,
            &[
                "init",
                "--quiet",
                "--template",
                template
                    .path()
                    .to_str()
                    .context("non-UTF8 release template directory")?,
            ],
        )?;
        git_run(&git, &["config", "core.hooksPath", "/dev/null"])?;
        git_run(&git, &["config", "commit.gpgSign", "false"])?;
        git_run(&git, &["config", "protocol.ext.allow", "never"])?;
        copy_identity(launch, &git)?;
        git_run(&git, &["remote", "add", "origin", &origin])?;
        Ok(Self {
            _directory: directory,
            git,
        })
    }
}

fn copy_identity(launch: &Git, repository: &Git) -> Result<()> {
    for key in ["user.name", "user.email"] {
        let Some(value) = git_optional(launch, &["config", key])? else {
            continue;
        };
        if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
            continue;
        }
        git_run(repository, &["config", key, &value])?;
    }
    Ok(())
}

fn usable_origin(url: &str) -> bool {
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_whitespace) {
        return false;
    }
    if let Some(repo) = Repo::of_origin(url) {
        return repo.slug().eq_ignore_ascii_case(REPOSITORY);
    }
    Path::new(url).is_absolute()
}

fn checkout(git: &Git, base: &str) -> Result<()> {
    // The base is a commit id. `--` would make `checkout --detach` treat it as a path.
    git_run(git, &["checkout", "--quiet", "--detach", base]).map(drop)
}

/// `git <args>` without the Launch directory's hooks or fsmonitor.
pub(super) fn git_run(repository: &Git, args: &[&str]) -> Result<String> {
    let mut full = vec!["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor="];
    full.extend_from_slice(args);
    repository.run(&full)
}

fn git_optional(repository: &Git, args: &[&str]) -> Result<Option<String>> {
    let mut full = vec!["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor="];
    full.extend_from_slice(args);
    repository.run_optional(&full)
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
