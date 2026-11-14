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

mod controls;
mod recovery;
mod selection;
mod summary;
mod workflow;

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
pub struct Request<'a> {
    pub version: Option<&'a str>,
    pub bump: Option<Bump>,
    pub with_experiment: bool,
    pub review: bool,
}

pub fn run(request: Request<'_>, choice: &mut Choice) -> Result<Option<Ended>, FailedRun> {
    let mut pr_url = None;
    let mut trace = None;
    let result = (|| -> Result<Option<Ended>> {
        outside_session()?;
        if request.review {
            ensure!(
                std::io::IsTerminal::is_terminal(&std::io::stdin()),
                "release --review requires a terminal on standard input"
            );
        }
        let explicit = request.version.map(Version::parse).transpose()?;
        let pass = request.version.is_none();
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
        let mut automatic_summary = None;
        let mut cut =
            match recovery::discover(git, &launch, &github, request.version, request.bump)? {
                recovery::Discovery::Blocked(reason) => {
                    if pass {
                        return skipped(&repo, reason);
                    }
                    anyhow::bail!("{reason}");
                }
                recovery::Discovery::Resume(cut) => {
                    if pass {
                        logs::started(logs::Work::ReleaseRun(&repo), choice);
                    }
                    *cut
                }
                recovery::Discovery::Fresh { previous } => {
                    if let (Some(wanted), Some(version)) = (&explicit, request.version) {
                        validate_version(git, wanted)?;
                        refuse_tag_on_origin(git, version)?;
                    }
                    let (base, _) = match selection::candidate(
                        git,
                        &github,
                        explicit.as_ref(),
                        previous.as_deref(),
                    )? {
                        selection::Candidate::Ready { base, previous } => (base, previous),
                        selection::Candidate::Skipped(reason) => return skipped(&repo, reason),
                    };
                    if pass {
                        logs::started(logs::Work::ReleaseRun(&repo), choice);
                    }
                    let version = if let Some(version) = request.version {
                        version.to_owned()
                    } else {
                        let current = main_version(git)?;
                        if let Some(bump) = request.bump {
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
                    validate_version(git, &Version::parse(&version)?)?;
                    refuse_tag_on_origin(git, &version)?;
                    ensure!(
                        git_run(&launch, &["tag", "--list", &format!("v{version}")])?.is_empty(),
                        "v{version} already exists locally"
                    );
                    recovery::Cut {
                        version,
                        base,
                        previous,
                        head: String::new(),
                        pr: None,
                        tagged: None,
                        published: false,
                        recut: false,
                        supersedes: None,
                        expected_branch: None,
                        expected_tag: None,
                    }
                }
            };
        if request.with_experiment && !cut.published {
            let pin = format!("refs/thirdshift/release-candidates/{}", cut.version);
            git_run(&launch, &["fetch", "--quiet", "origin", &cut.base])?;
            git_run(&launch, &["update-ref", &pin, &cut.base])?;
        }
        if cut.head.is_empty() {
            cut.commit(git)?;
        }
        workflow::finish(
            git,
            &github,
            &mut cut,
            &request,
            &mut workflow::Notes {
                repo: &repo,
                choice,
                trace: &mut trace,
            },
            automatic_summary,
            &mut pr_url,
        )?;
        Ok(Some(Ended {
            pr_url: cut.pr.context("release has no PR")?.url,
            version: cut.version,
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

/// A fresh cut never reuses a tag already on origin, published or not.
fn refuse_tag_on_origin(git: &Git, version: &str) -> Result<()> {
    let tag = format!("refs/tags/v{version}");
    ensure!(
        git_run(git, &["ls-remote", "--tags", "origin", &tag])?.is_empty(),
        "v{version} already exists on origin"
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

fn bump_identity(snapshot: &crate::github::PrSnapshot, branch: &str) -> Result<()> {
    ensure!(
        !snapshot.from_fork && snapshot.pr.head == branch && snapshot.pr.base == "main",
        "{} changed release PR identity; inspect it before retrying",
        snapshot.pr.url
    );
    ensure!(
        !snapshot.pr.is_draft,
        "{} is a draft; review it and run gh pr ready {}, then retry thirdshift release {}",
        snapshot.pr.url,
        snapshot.pr.url,
        branch.trim_start_matches("release-")
    );
    Ok(())
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
    bump_identity(&snapshot, branch)?;
    ensure!(
        snapshot.pr.state == PrState::Open && snapshot.head_commit == head,
        "release bump pull request is no longer open and ready at the validated head; the Day shift must review it"
    );
    ensure!(
        snapshot.mergeable != Mergeable::No,
        "release bump pull request has a merge conflict"
    );
    Ok(snapshot.mergeable)
}

/// Names the failed jobs and the run, or the bare conclusion when the
/// run's details cannot be read, with the rerun that recovers it.
fn release_failure(github: &GitHub, run: &crate::github::release::ReleasePushRun) -> String {
    let rerun = format!("recover with gh run rerun --failed {}", run.id);
    match github.release_workflow_jobs(run.id) {
        Ok(detail) => {
            let failed = detail
                .jobs
                .iter()
                .filter(|job| {
                    !matches!(
                        job.conclusion,
                        Conclusion::Success | Conclusion::Skipped | Conclusion::Neutral
                    )
                })
                .map(|job| job.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!("{failed} at {}; {rerun}", detail.url)
        }
        Err(error) => format!(
            "{} (job details unavailable: {error:#}); {rerun}",
            run.conclusion.as_deref().unwrap_or("no conclusion")
        ),
    }
}

fn wait_for_release(github: &GitHub, head: &str) -> Result<()> {
    let mut line = poll::WaitingLine::new();
    let started = Instant::now();
    loop {
        let run = github.release_push_run("release.yml", "", head)?;
        if let Some(run) = &run {
            if run.status == "completed" {
                if run.conclusion.as_deref() != Some("success") {
                    anyhow::bail!(
                        "release workflow failed: {}; tag is left for the Day shift",
                        release_failure(github, run)
                    );
                }
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
        let canonical = git_run(launch, &["config", "remote.origin.url"])?;
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
        let repository = directory.path().join("repository");
        fs::create_dir(&repository)?;
        let git = Git::new(repository);
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
        git_run(&git, &["remote", "add", "origin", &canonical])?;
        if origin != canonical {
            git_run(
                &git,
                &["config", &format!("url.{origin}.insteadOf"), &canonical],
            )?;
        }
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
