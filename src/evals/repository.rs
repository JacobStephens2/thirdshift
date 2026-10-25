//! Import only base ancestry, carry the reviewed change as a patch, and verify
//! every ref before exposing the throwaway repository to a Trial.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

use super::format::{Session, State, Task};
use crate::process::{self, Control, Interruption};

#[derive(Serialize)]
pub(super) struct Prepared {
    pub repository: PathBuf,
    origin: Option<PathBuf>,
}

pub(super) fn prepare(directory: &Path, task: &Task, destination: &Path) -> Result<Prepared> {
    prepare_repository(directory, task, destination, None)
}

pub(super) fn prepare_trial(directory: &Path, task: &Task, destination: &Path) -> Result<Prepared> {
    let inputs = task.trial.as_ref().context("missing Trial inputs")?;
    git(directory, &["check-ref-format", "--branch", &inputs.base])?;
    git(directory, &["check-ref-format", "--branch", &inputs.branch])?;
    ensure!(
        inputs.base != inputs.branch,
        "Trial Base branch and Issue branch must differ"
    );
    prepare_repository(directory, task, destination, Some(inputs))
}

fn prepare_repository(
    directory: &Path,
    task: &Task,
    destination: &Path,
    trial: Option<&crate::trial::Inputs>,
) -> Result<Prepared> {
    if task.state != State::Approved {
        bail!("proposed Eval tasks cannot be prepared or run");
    }
    if destination.try_exists()? {
        bail!("Eval destination already exists: {}", destination.display());
    }
    let destination = std::env::current_dir()?.join(destination);
    let parent = destination
        .parent()
        .context("Eval destination needs a parent")?
        .canonicalize()?;
    let destination = parent.join(
        destination
            .file_name()
            .context("Eval destination needs a name")?,
    );
    let source = directory
        .join(&task.repository)
        .canonicalize()
        .context("Eval repository must be a local directory")?;
    let source_path = source
        .to_str()
        .context("Eval repository path must be UTF-8")?;
    // Full IDs plus disabled replacement objects prevent a moving ref from
    // changing the history or patch after the manifest was written.
    for id in [&task.base, &task.change] {
        git(&source, &["cat-file", "-e", &format!("{id}^{{commit}}")])?;
    }
    git(
        &source,
        &["merge-base", "--is-ancestor", &task.base, &task.change],
    )?;
    let object_format = git(&source, &["rev-parse", "--show-object-format"])?;
    let temporary = tempfile::Builder::new()
        .prefix(".thirdshift-eval-")
        .tempdir_in(&parent)?;
    let repo = temporary.path();
    git(
        repo,
        &[
            "init",
            "-q",
            "--template=",
            &format!("--object-format={object_format}"),
        ],
    )?;
    git(
        repo,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            "--no-recurse-submodules",
            source_path,
            &task.base,
        ],
    )?;
    git(repo, &["checkout", "-q", "--detach", &task.base])?;
    let patch = bytes(
        &source,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--binary",
            "--full-index",
            "--no-color",
            "--unified=3",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--submodule=short",
            "--ignore-submodules=none",
            &task.base,
            &task.change,
            "--",
        ],
        None,
    )?;
    if !patch.is_empty() {
        bytes(repo, &["apply", "--index", "--binary", "-"], Some(&patch))?;
    }
    copy_inputs(directory, task, repo)?;
    prune(repo)?;
    verify(repo, &task.base)?;
    let origin = if task.session == Session::SpecReview {
        let relative = ".git/eval-origin.git";
        let bare = repo.join(relative);
        git(
            repo,
            &[
                "init",
                "-q",
                "--bare",
                "--template=",
                &format!("--object-format={object_format}"),
                relative,
            ],
        )?;
        git(
            &bare,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                source_path,
                &format!("{}:refs/heads/base", task.base),
            ],
        )?;
        git(&bare, &["symbolic-ref", "HEAD", "refs/heads/base"])?;
        prune(&bare)?;
        verify(&bare, &task.base)?;
        let final_origin = destination.join(relative);
        git(
            repo,
            &[
                "remote",
                "add",
                "origin",
                final_origin
                    .to_str()
                    .context("Eval origin path must be UTF-8")?,
            ],
        )?;
        Some((bare, final_origin))
    } else {
        None
    };
    if let Some(inputs) = trial {
        git(repo, &["branch", &inputs.base, &task.base])?;
        git(repo, &["checkout", "-b", &inputs.branch])?;
        // Commit the staged patch only. Copied inputs may overlay tracked
        // worktree files without belonging to the reviewed change.
        git(
            repo,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Eval",
                "-c",
                "user.email=eval@example.invalid",
                "commit",
                "--allow-empty",
                "-m",
                "Eval change",
            ],
        )?;
        if let Some((bare, _)) = &origin {
            // The configured remote names the final destination, which does
            // not exist until publication. Push to the staged origin directly.
            git(
                repo,
                &[
                    "push",
                    bare.to_str().context("Eval origin path must be UTF-8")?,
                    &format!("refs/heads/{0}:refs/heads/{0}", inputs.base),
                    &format!("refs/heads/{0}:refs/heads/{0}", inputs.branch),
                ],
            )?;
        }
        // This repository is uniquely owned, unlike ordinary shared worktrees.
        fs::create_dir_all(repo.join(".git/info"))?;
        fs::write(
            repo.join(".git/info/exclude"),
            format!("/{}\n", crate::prompt::ISSUE_FILE),
        )?;
    }
    fs::rename(repo, &destination).context("could not publish prepared Eval repository")?;
    Ok(Prepared {
        repository: destination,
        origin: origin.map(|(_, final_origin)| final_origin),
    })
}

fn copy_inputs(directory: &Path, task: &Task, repo: &Path) -> Result<()> {
    let root = directory.canonicalize()?;
    for (destination, source) in &task.inputs {
        let source = root.join(source).canonicalize()?;
        if !source.starts_with(&root) || !source.is_file() {
            bail!("Eval input must be a file inside its task folder");
        }
        let mut target = repo.to_path_buf();
        let parts: Vec<_> = destination.split('/').collect();
        for (index, part) in parts.iter().enumerate() {
            target.push(part);
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_symlink() => bail!(
                    "Eval input destination crosses a symlink: {}",
                    target.display()
                ),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if index + 1 < parts.len() {
                        fs::create_dir(&target)?;
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        fs::copy(source, target)?;
    }
    Ok(())
}

fn prune(repo: &Path) -> Result<()> {
    git(repo, &["reflog", "expire", "--expire=now", "--all"])?;
    git(repo, &["repack", "-ad"])?;
    git(repo, &["prune", "--expire=now"])?;
    Ok(())
}

pub(super) fn verify(repo: &Path, base: &str) -> Result<()> {
    let expected = git(repo, &["rev-list", base])?;
    let actual = git(repo, &["rev-list", "--all", "HEAD"])?;
    let expected: BTreeSet<_> = expected.lines().collect();
    let actual: BTreeSet<_> = actual.lines().collect();
    if actual != expected || git(repo, &["rev-parse", "HEAD"])? != base {
        bail!("Eval preparation refused: history beyond base {base} survives");
    }
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(bytes(repo, args, None)?)
        .map(|s| s.trim().into())
        .context("git output is not UTF-8")
}

fn bytes(repo: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    // Prepared repositories must not inherit host hooks, templates, replacement
    // refs or Git directory overrides. Local fetching needs no host credentials.
    let output = process::output(
        Command::new("git")
            .current_dir(repo)
            .args(["-c", "gc.auto=0", "-c", "protocol.file.allow=always"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES"),
        input,
        Control {
            name: "Eval git",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
    )?;
    if !output.status.success() {
        bail!(
            "Eval git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests;
