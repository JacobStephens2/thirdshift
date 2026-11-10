//! Draft complete tasks atomically; approval remains a separate Day-shift decision.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{TestSelection, git, layout};
use crate::evals::{
    format::{self, Environment, Provenance, Session, State, Task},
    suite,
};
use crate::github::{
    GitHub,
    evals::{Candidate, Ticket},
};
use crate::issue::{IssueUrl, Repo};

#[derive(Default, Serialize)]
pub(in crate::evals) struct Report {
    proposals: Vec<Proposal>,
    skipped: Vec<Skipped>,
}

#[derive(Serialize)]
struct Proposal {
    number: u64,
    url: String,
    task: String,
    kind: &'static str,
    non_test_lines: usize,
    #[serde(flatten)]
    tests: TestSelection,
}

#[derive(Serialize)]
struct Skipped {
    number: u64,
    reason: String,
}

pub(in crate::evals) fn propose(source: &Path, limit: u32) -> Result<Report> {
    let source = source.canonicalize()?;
    let origin = git(&source, &["remote", "get-url", "origin"])?;
    let repo = Repo::of_origin(&origin).context("process proposals require a GitHub origin")?;
    let candidates = GitHub::new().process_candidates(&repo, limit)?;
    let directory = crate::config::home()?.join(".thirdshift/evals/suites/process");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory.join("tasks"))?;
    let manifest = directory.join("suite.toml");
    if !manifest.try_exists()? {
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        file.write_all(suite_manifest()?.as_bytes())?;
        file.persist_noclobber(&manifest)?;
    }
    let _lock = suite::lock(&directory)?;
    let mut report = Report::default();
    for pr in candidates {
        match draft(&source, &origin, &repo, &directory, &pr) {
            Ok(proposals) => report.proposals.extend(proposals),
            Err(error) => {
                crate::interrupt::check()?;
                report.skipped.push(Skipped {
                    number: pr.number,
                    reason: format!("{error:#}"),
                });
            }
        }
    }
    Ok(report)
}

fn draft(
    source: &Path,
    origin: &str,
    repo: &Repo,
    suite: &Path,
    pr: &Candidate,
) -> Result<Vec<Proposal>> {
    let tickets = pr.tickets()?;
    format::commit_id(&pr.merge)?;
    let staging = tempfile::Builder::new()
        .prefix(".process-proposal-")
        .tempdir_in(suite)?;
    let retained = staging.path().join("repository");
    fs::create_dir(&retained)?;
    let object_format = git(source, &["rev-parse", "--show-object-format"])?;
    git(
        &retained,
        &[
            "init",
            "-q",
            "--template=",
            &format!("--object-format={object_format}"),
        ],
    )?;
    if git(
        &retained,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            source.to_str().context("non-UTF-8 source path")?,
            &pr.merge,
        ],
    )
    .is_err()
    {
        crate::interrupt::check()?;
        // Unlike local preparation, discovery may fetch a recent merge not yet
        // present in the operator's clone, using the host's Git credentials.
        crate::git::Git::new(&retained).run(&["fetch", "--no-tags", origin, &pr.merge])?;
    }
    let base = super::history::base(&retained, source, origin, pr)?;
    layout::reject_inline_changes(&retained, &base, &pr.merge)?;
    let (tests, non_test_lines) = layout::selection(&retained, &base, &pr.merge)?;
    tests.validate()?;
    ensure!(non_test_lines > 0, "pull request changes no source");
    git(&retained, &["update-ref", "refs/heads/base", &base])?;
    git(&retained, &["update-ref", "refs/heads/change", &pr.merge])?;
    let mut proposals = Vec::new();
    for ticket in tickets {
        let issue = IssueUrl::parse(&ticket.url)?;
        let name = format!(
            "pr-{:x}-{}-ticket-{}",
            Sha256::digest(repo.slug().to_ascii_lowercase()),
            pr.number,
            issue.number
        );
        let destination = suite.join("tasks").join(&name);
        if destination.try_exists()? {
            Task::load(&destination)?;
        } else {
            publish(&retained, suite, &destination, pr, ticket, &base, &tests)?;
        }
        proposals.push(Proposal {
            number: pr.number,
            url: pr.url.clone(),
            task: name,
            kind: ticket.kind,
            non_test_lines,
            tests: tests.clone(),
        });
    }
    Ok(proposals)
}

fn publish(
    repository: &Path,
    suite: &Path,
    destination: &Path,
    pr: &Candidate,
    ticket: &Ticket,
    base: &str,
    tests: &TestSelection,
) -> Result<()> {
    let staging = tempfile::Builder::new()
        .prefix(".process-task-")
        .tempdir_in(suite)?;
    let retained = staging.path().join("repository");
    crate::git::Git::new(suite).run(&[
        "clone",
        "--quiet",
        "--no-checkout",
        "--no-hardlinks",
        repository.to_str().context("non-UTF-8 retained path")?,
        retained.to_str().context("non-UTF-8 task path")?,
    ])?;
    git(&retained, &["update-ref", "refs/heads/base", base])?;
    git(&retained, &["update-ref", "refs/heads/change", &pr.merge])?;
    git(&retained, &["remote", "remove", "origin"])?;
    let task = Task {
        format: 1, state: State::Proposed, session: Session::Process,
        provenance: Some(Provenance::Real), environment: Some(Environment::Host),
        repository: "repository".into(), base: base.into(), change: pr.merge.clone(),
        criteria: "Implement the Ticket and pass its reference tests. Review the draft selectors before approval.".into(),
        checks: Vec::new(), inputs: BTreeMap::from([(".thirdshift-issue.md".into(), "issue.md".into())]),
        trial: Some(crate::trial::Inputs { issue_url: ticket.url.clone(), base: pr.base_ref_name.clone(), branch: pr.head_ref_name.clone(), pr_url: String::new() }),
        difficulty: None,
        process_tests: Some(tests.clone()),
    };
    fs::write(
        staging.path().join("task.toml"),
        toml::to_string_pretty(&task)?,
    )?;
    fs::write(
        staging.path().join("issue.md"),
        format!("# {}\n\n{}\n\n{}\n", ticket.title, ticket.url, ticket.body),
    )?;
    Task::load(staging.path())?;
    fs::rename(staging.path(), destination)?;
    Ok(())
}

fn suite_manifest() -> Result<String> {
    let mut document = crate::evals::import::suite_manifest(3).parse::<toml_edit::DocumentMut>()?;
    let skills = document["dependencies"]["factory_skills"]
        .as_array_mut()
        .context("missing Factory skills")?;
    skills.push("skills/thirdshift-implement/");
    skills.push("skills/thirdshift-security-audit/");
    let mut processes = toml_edit::Array::new();
    for path in [
        "processes/",
        "src/processes.rs",
        "src/processes/",
        "src/implement.rs",
        "src/security.rs",
        "src/security/",
    ] {
        processes.push(path);
    }
    document["dependencies"]["builtin_processes"] = toml_edit::value(processes);
    Ok(document.to_string())
}
