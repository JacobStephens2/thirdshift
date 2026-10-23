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
    if input.merge_commit.len() != 40
        || !input
            .merge_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("Scoring packet has an invalid merge commit");
    }
    let repository = dir.join("repository");
    fs::create_dir_all(&repository)?;
    let git = Git::new(&repository);
    git.run(&["init", "--quiet"])?;
    // Fetch only the captured merge and its parents. No branch, tag, remote
    // configuration or object from a later commit ever enters this clone.
    git.run(&[
        "fetch",
        "--no-tags",
        "--depth=2",
        &format!("https://github.com/{}.git", pr.repository),
        &input.merge_commit,
    ])?;
    git.run(&["checkout", "--detach", "FETCH_HEAD"])?;
    let ancestry = git.run(&["rev-list", "--parents", "-n", "1", "HEAD"])?;
    let diff = if let Some(parent) = ancestry.split_whitespace().nth(1) {
        git.run(&[
            "diff",
            "--binary",
            "--no-ext-diff",
            "--no-textconv",
            parent,
            "HEAD",
            "--",
        ])?
    } else {
        git.run(&[
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--binary",
            "HEAD",
            "--",
        ])?
    };
    // A detached checkout alone still exposes commit messages, identities and
    // fetch information. Only the checked-out files belong to the packet.
    fs::remove_dir_all(repository.join(".git"))?;
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
    text.lines()
        .filter(|line| {
            let line = line.trim_start_matches(['+', '-', ' ']);
            !line.contains(built_with::MARKER)
                && !line.starts_with("Built with ")
                && !line.contains("Generated with Claude Code")
                && !line.contains("Generated with [Claude Code]")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
