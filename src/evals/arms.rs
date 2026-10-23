//! Prepare both Arms before exposing their fixed binaries to the Trial runner.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::format;
use crate::harness::Harness;
use crate::process::{self, Control, Interruption};

pub(crate) struct Requested {
    commit: String,
    harness: Harness,
}

impl Requested {
    pub fn new(commit: &str, harness: &str) -> Result<Self> {
        format::commit_id(commit)?;
        Ok(Self {
            commit: commit.into(),
            harness: Harness::named(harness).context("unknown Arm Harness")?,
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct FixedHarness {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Arm {
    pub commit: String,
    pub binary: PathBuf,
    pub harness: FixedHarness,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Prepared {
    format: u32,
    pub(super) baseline: Arm,
    pub(super) candidate: Arm,
}

pub(super) fn prepare(repository: &Path, arms: [Requested; 2], record: &Path) -> Result<Prepared> {
    if record.try_exists()? {
        bail!("Arm record already exists: {}", record.display());
    }
    let [baseline, candidate] = arms;
    // Resolve all Harnesses at the start, before either potentially long build.
    // The same Harness is resolved only once even when both Arms use it.
    let baseline_harness = fixed_harness(baseline.harness)?;
    let candidate_harness = if baseline.harness == candidate.harness {
        baseline_harness.clone()
    } else {
        fixed_harness(candidate.harness)?
    };
    let builds = crate::config::home()?.join(".thirdshift/evals/builds");
    fs::create_dir_all(&builds)?;
    let builds = builds.canonicalize()?;
    let prepared = Prepared {
        format: 1,
        baseline: Arm {
            binary: build(repository, &builds, &baseline.commit)?,
            commit: baseline.commit,
            harness: baseline_harness,
        },
        candidate: Arm {
            binary: build(repository, &builds, &candidate.commit)?,
            commit: candidate.commit,
            harness: candidate_harness,
        },
    };
    let json = serde_json::to_string_pretty(&prepared)?;
    let parent = record
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(json.as_bytes())?;
    temporary
        .persist_noclobber(record)
        .with_context(|| format!("can't publish Arm record {}", record.display()))?;
    Ok(prepared)
}

fn fixed_harness(harness: Harness) -> Result<FixedHarness> {
    let name = harness.name();
    let path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| executable(path))
        .with_context(|| format!("Arm Harness {name} is not on PATH"))?
        .canonicalize()?;
    let output = checked(
        Command::new(&path)
            .arg("--version")
            .envs(harness.adapter().environment().iter().copied()),
        "Arm Harness version",
    )?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    let version = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    }
    .to_owned();
    if version.is_empty() {
        bail!("Arm Harness {name} reported an empty version");
    }
    Ok(FixedHarness {
        name: name.into(),
        path,
        version,
    })
}

fn executable(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn build(repository: &Path, builds: &Path, commit: &str) -> Result<PathBuf> {
    let git = crate::git::Git::new(repository);
    if git.run(&["--no-replace-objects", "cat-file", "-t", commit])? != "commit" {
        bail!("Arm {commit} is not a commit");
    }
    let lock = fs::File::create(builds.join(format!("{commit}.lock")))?;
    lock.lock()?;
    let cached = builds.join(commit);
    let binary = cached.join("thirdshift");
    if executable(&binary) {
        return Ok(binary);
    }
    if cached.try_exists()? {
        bail!("invalid Arm build cache entry: {}", cached.display());
    }
    let temporary = tempfile::Builder::new()
        .prefix(".arm-build-")
        .tempdir_in(builds)?;
    let worktree = temporary.path().join("worktree");
    let worktree_path = worktree
        .to_str()
        .context("Arm worktree path must be UTF-8")?;
    git.run(&[
        "--no-replace-objects",
        "-c",
        "core.hooksPath=/dev/null",
        "worktree",
        "add",
        "--detach",
        worktree_path,
        commit,
    ])?;
    let target = temporary.path().join("target");
    let result = checked(
        Command::new("cargo")
            .args(["build", "--release", "--locked"])
            .current_dir(&worktree)
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env("CARGO_TARGET_DIR", &target),
        "Arm build",
    );
    // Cleanup must still finish when ordinary work was interrupted.
    let cleanup = git
        .completion()
        .run(&["worktree", "remove", "--force", worktree_path]);
    result.with_context(|| format!("could not build Arm {commit}"))?;
    cleanup?;
    let built = target.join("release/thirdshift");
    if !executable(&built) {
        bail!(
            "Arm build {commit} produced no executable at {}",
            built.display()
        );
    }
    let published = temporary.path().join("published");
    fs::create_dir(&published)?;
    fs::copy(built, published.join("thirdshift"))?;
    fs::rename(published, cached)?;
    Ok(binary)
}

fn checked(command: &mut Command, name: &str) -> Result<Output> {
    let output = process::output(
        command,
        None,
        Control {
            name,
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
    )?;
    if !output.status.success() {
        bail!(
            "{name} failed ({}): {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    Ok(output)
}
