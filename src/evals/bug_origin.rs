//! Publish complete, private proposals using the same format as approved Evals.
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::DirBuilderExt;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::format::{self, Session, State, Task};
use crate::git::Git;
use crate::github::scoring::origins::ReviewedPullRequest;
use crate::issue::IssueUrl;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Reference {
    pub suite: String,
    pub task: String,
}

pub(crate) fn propose(
    bug: &IssueUrl,
    pr: &IssueUrl,
    review: ReviewedPullRequest,
    reason: &str,
) -> Result<Reference> {
    let reference = Reference {
        suite: "code-review".into(),
        // GitHub repository names allow dots; Eval names do not. Hash the
        // complete identity to keep valid, distinct names across repositories.
        task: format!(
            "bug-{:x}-pr-{}",
            Sha256::digest(format!(
                "{}#{}",
                bug.repo_slug().to_ascii_lowercase(),
                bug.number
            )),
            pr.number
        ),
    };
    format::name(&reference.task)?;
    let suite = crate::config::home()?
        .join(".thirdshift/evals/suites")
        .join(&reference.suite);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(suite.join("tasks"))?;
    let manifest = suite.join("suite.toml");
    if !manifest.try_exists()? {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(&suite)?;
        file.write_all(super::import::suite_manifest(3).as_bytes())?;
        file.persist_noclobber(&manifest)?;
    }
    super::suite::Manifest::load(&suite)?;
    let destination = suite.join("tasks").join(&reference.task);
    if destination.try_exists()? {
        Task::load(&destination)?;
        return Ok(reference);
    }
    format::commit_id(&review.base)?;
    format::commit_id(&review.head)?;
    let staging = tempfile::Builder::new()
        .prefix(".bug-origin-")
        .tempdir_in(&suite)?;
    let repository = staging.path().join("repository");
    fs::create_dir(&repository)?;
    let git = Git::new(&repository);
    git.run(&["init", "--quiet", "--template="])?;
    git.run(&[
        "fetch",
        "--no-tags",
        "--no-write-fetch-head",
        "--no-recurse-submodules",
        &format!("https://github.com/{}.git", pr.repo_slug()),
        &review.base,
        &review.head,
    ])?;
    let base = git.run(&["merge-base", &review.base, &review.head])?;
    ensure!(base != review.head, "Bug-origin reviewed diff is empty");
    // Keep both objects reachable for future local-only preparation.
    git.run(&["update-ref", "refs/heads/base", &base])?;
    git.run(&["update-ref", "refs/heads/change", &review.head])?;
    fs::write(staging.path().join("issue.md"), review.input)?;
    let task = Task {
        format: 1,
        state: State::Proposed,
        session: Session::CodeReview,
        repository: "repository".into(),
        base,
        change: review.head,
        criteria: format!(
            "Draft catch criterion: catch the bug tracked by {} in this reviewed diff.\n{}\nReport no wrong findings. Refine this criterion before approval.",
            bug.url, reason
        ),
        checks: Vec::new(),
        inputs: BTreeMap::from([(".thirdshift-issue.md".into(), "issue.md".into())]),
        trial: Some(review.trial),
    };
    fs::write(
        staging.path().join("task.toml"),
        toml::to_string_pretty(&task)?,
    )?;
    Task::load(staging.path())?;
    fs::rename(staging.path(), destination)?;
    Ok(reference)
}

/// Approval is an independent Day-shift decision; only pending proposals leave.
pub(crate) fn withdraw(reference: &Reference) -> Result<()> {
    format::name(&reference.suite)?;
    format::name(&reference.task)?;
    let directory = crate::config::home()?
        .join(".thirdshift/evals/suites")
        .join(&reference.suite)
        .join("tasks")
        .join(&reference.task);
    if directory.try_exists()? && Task::load(&directory)?.state == State::Proposed {
        fs::remove_dir_all(directory)?;
    }
    Ok(())
}
