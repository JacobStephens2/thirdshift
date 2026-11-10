//! Select the whole PR's range for merge, squash and rebase merges. Rewritten
//! rebase commits are identified by their complete patch, not their final parent.

use std::path::Path;

use anyhow::{Result, bail, ensure};

use super::{bytes, git};
use crate::evals::format;
use crate::github::evals::Candidate;

pub(super) fn base(
    repository: &Path,
    source: &Path,
    origin: &str,
    pr: &Candidate,
) -> Result<String> {
    let parents = git(repository, &["rev-list", "--parents", "-n", "1", &pr.merge])?;
    let parents: Vec<_> = parents.split_whitespace().skip(1).collect();
    let Some(first) = parents.first() else {
        bail!("merged PR has no base parent")
    };
    if parents.len() > 1 {
        return Ok((*first).into());
    }
    let Some(original) = &pr.original else {
        return Ok((*first).into());
    };
    format::commit_id(&original.head)?;
    format::commit_id(&original.base)?;
    ensure!(original.commits > 0, "PR commit list is empty");
    if git(repository, &["cat-file", "-e", &original.head]).is_err() {
        crate::interrupt::check()?;
        if git(
            repository,
            &[
                "fetch",
                "--no-tags",
                source
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-UTF-8 source"))?,
                &original.head,
            ],
        )
        .is_err()
        {
            crate::interrupt::check()?;
            crate::git::Git::new(repository).run(&[
                "fetch",
                "--no-tags",
                origin,
                &original.head,
            ])?;
        }
    }
    if original.head == pr.merge {
        return Ok(original.base.clone());
    }
    // A PR can merge updated main before it is squashed. Compare only the PR
    // changes after the shared target ancestry, excluding those main changes.
    let original_base = git(repository, &["merge-base", first, &original.head])?;
    let expected = patch_id(repository, &original_base, &original.head)?;
    if patch_id(repository, first, &pr.merge)? == expected {
        return Ok((*first).into());
    }
    let rebased_base = git(
        repository,
        &["rev-parse", &format!("{}~{}", pr.merge, original.commits)],
    )?;
    ensure!(
        patch_id(repository, &rebased_base, &pr.merge)? == expected,
        "cannot identify the complete merged PR range; author this task manually"
    );
    Ok(rebased_base)
}

fn patch_id(repository: &Path, base: &str, change: &str) -> Result<Vec<u8>> {
    let patch = bytes(
        repository,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--binary",
            "--full-index",
            base,
            change,
            "--",
        ],
        None,
    )?;
    let id = bytes(repository, &["patch-id", "--stable"], Some(&patch))?;
    let id = id
        .split(|byte| byte.is_ascii_whitespace())
        .next()
        .unwrap_or_default();
    ensure!(!id.is_empty(), "PR has no patch");
    Ok(id.to_vec())
}
