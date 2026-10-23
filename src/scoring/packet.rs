//! The Grader's only input: blind text and a merge-time repository snapshot.
//! Attribution stays in the sample; neither it nor Git metadata is handed on.
use std::fs;
use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use super::PullRequest;
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::built_with;

#[derive(Serialize)]
pub(crate) struct Text {
    pub title: String,
    pub body: String,
}

#[derive(Serialize)]
pub(crate) struct Check {
    pub name: String,
    pub conclusion: Option<String>,
}

#[derive(Serialize)]
struct Packet {
    issues: Vec<Text>,
    pull_request: Text,
    diff: String,
    ci: Vec<Check>,
}

pub(super) fn write(github: &GitHub, pr: &PullRequest, dir: &Path) -> Result<()> {
    let input = github.scoring_packet(pr)?;
    for commit in [&input.merge_commit, &input.base_commit] {
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("Scoring packet has an invalid commit");
        }
    }
    let repository = dir.join("repository");
    fs::create_dir_all(&repository)?;
    let git = Git::new(&repository);
    git.run(&["init", "--quiet"])?;
    // Fetch only the captured merge and PR base. No branch, tag, remote
    // configuration or object from a later commit ever enters this clone.
    git.run(&[
        "fetch",
        "--no-tags",
        "--depth=1",
        &format!("https://github.com/{}.git", pr.repository),
        &input.merge_commit,
        &input.base_commit,
    ])?;
    git.run(&["checkout", "--detach", &input.merge_commit])?;
    // The PR's captured base covers the whole change for merge, squash and
    // rebase merges; a rebased mergeCommit's parent covers only its last commit.
    let diff = git.run(&[
        "diff",
        "--binary",
        "--no-ext-diff",
        "--no-textconv",
        &input.base_commit,
        "HEAD",
        "--",
    ])?;
    // A detached checkout alone still exposes commit messages, identities and
    // fetch information. Only the checked-out files belong to the packet.
    fs::remove_dir_all(repository.join(".git"))?;
    blind_repository(&repository)?;
    let packet = Packet {
        issues: input.issues.into_iter().map(blind).collect(),
        pull_request: blind(input.pull_request),
        diff: strip_attribution(&diff),
        ci: input
            .ci
            .into_iter()
            .map(|check| Check {
                name: strip_attribution(&check.name),
                conclusion: check.conclusion,
            })
            .collect(),
    };
    fs::write(dir.join("packet.json"), serde_json::to_vec_pretty(&packet)?)?;
    Ok(())
}

fn blind(text: Text) -> Text {
    Text {
        title: strip_attribution(&text.title),
        body: strip_attribution(&text.body),
    }
}

fn strip_attribution(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|line| {
            let line = line.trim_start_matches(['+', '-', ' ', '\t', '>', '*', '[', '🤖']);
            line.trim() != built_with::MARKER
                && !line.starts_with("Built with ")
                && !line.starts_with("Generated with Claude Code")
                && !line.starts_with("Generated with [Claude Code]")
        })
        .collect()
}

/// Inspect UTF-8 text without NUL bytes. Symlinks keep their merge-time targets;
/// following them could read or rewrite a file outside the packet.
fn blind_repository(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            blind_repository(&entry.path())?;
        } else if kind.is_file()
            && let Ok(text) = String::from_utf8(fs::read(entry.path())?)
            && !text.contains('\0')
        {
            let blinded = strip_attribution(&text);
            if blinded != text {
                fs::write(entry.path(), blinded)?;
            }
        }
    }
    Ok(())
}
