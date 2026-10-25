//! The #429 replay manifests have a stable shape; private grading prose is
//! supplied in an explicit index because replay.py does not define its format.
//! Build and validate everything before publishing complete Eval task folders.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::format::{self, Session, State, Task};
use super::suite;

#[derive(Deserialize)]
struct Forward {
    timing_bugs: Vec<ForwardBug>,
    spec_reviews: Vec<Spec>,
}

#[derive(Deserialize)]
struct Reverse {
    codex_bugs: Vec<ReverseBug>,
}

#[derive(Deserialize)]
struct ForwardBug {
    id: BugId,
    introducing_pr: PullRequest,
}

#[derive(Deserialize)]
struct ReverseBug {
    id: BugId,
    introducing_prs: Vec<PullRequest>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BugId {
    Number(u64),
    Text(String),
}

impl BugId {
    fn key(&self) -> String {
        match self {
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }
}

#[derive(Deserialize)]
struct PullRequest {
    number: u64,
    merge_base: String,
    head_as_merged: String,
    implements_issue: u64,
}

#[derive(Deserialize)]
struct Spec {
    repo: String,
    spec_issue: u64,
    spec_issue_url: String,
    spec_pr_url: String,
    base_branch: String,
    spec_branch: String,
    merge_base: String,
    pre_review_head: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Grades {
    bugs: BTreeMap<String, String>,
    specs: BTreeMap<String, Vec<Finding>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    finding: String,
    score: FindingScore,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FindingScore {
    Real,
    Wrong,
}

#[derive(Deserialize)]
struct Issue {
    title: String,
    body: String,
    parent: Option<u64>,
    tickets: Vec<u64>,
}

#[derive(Serialize, Default)]
pub(super) struct Imported {
    imported: usize,
    skipped: usize,
}

pub(super) fn replays(home: &Path) -> Result<Imported> {
    let source = home.join(".thirdshift/replays");
    let dated = source.join("2026-10-09");
    let forward: Forward = json(&source.join("inputs-2026-10-07/manifest.json"))?;
    let reverse: Reverse = json(&dated.join("inputs/manifest.json"))?;
    let grades: Grades = json(&dated.join("grading/eval-import.json"))?;
    let root = home.join(".thirdshift/evals/suites");
    let parent = root.parent().context("Eval storage needs a parent")?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".replay-import-")
        .tempdir_in(parent)?;
    for (name, trials) in [("code-review", 3), ("spec-review", 1)] {
        let directory = staging.path().join(name);
        fs::create_dir_all(directory.join("tasks"))?;
        fs::write(directory.join("suite.toml"), suite_manifest(trials))?;
    }
    for bug in forward.timing_bugs {
        bug_task(
            home,
            &dated,
            staging.path(),
            &grades,
            &bug.id,
            bug.introducing_pr,
        )?;
    }
    for bug in reverse.codex_bugs {
        if bug.introducing_prs.is_empty() {
            bail!("bug {} has no introducing PRs", bug.id.key());
        }
        for pr in bug.introducing_prs {
            bug_task(home, &dated, staging.path(), &grades, &bug.id, pr)?;
        }
    }
    for spec in forward.spec_reviews {
        format::name(&spec.repo)?;
        let name = format!("{}-{}", spec.repo, spec.spec_issue);
        let findings = grades
            .specs
            .get(&name)
            .with_context(|| format!("missing Spec grades for {name}"))?;
        let mut criteria =
            String::from("Catch the known real findings and avoid wrong findings.\n");
        for finding in findings {
            if finding.finding.trim().is_empty() {
                bail!("blank Spec finding for {name}");
            }
            let score = match finding.score {
                FindingScore::Real => "real",
                FindingScore::Wrong => "wrong",
            };
            criteria.push_str(&format!("\n{score}: {}\n", finding.finding));
        }
        write_task(
            staging.path(),
            "spec-review",
            &name,
            Task {
                format: 1,
                state: State::Approved,
                session: Session::SpecReview,
                repository: repository(home, &spec.repo)?,
                base: spec.merge_base,
                change: spec.pre_review_head,
                criteria,
                checks: Vec::new(),
                inputs: inputs(),
                trial: Some(crate::trial::Inputs {
                    issue_url: spec.spec_issue_url,
                    pr_url: spec.spec_pr_url,
                    base: spec.base_branch,
                    branch: spec.spec_branch,
                }),
            },
            &context(&dated, &spec.repo, spec.spec_issue)?,
        )?;
    }
    // Validate destinations before writing either suite. A repeated import
    // preserves edits to valid existing tasks and declarations.
    for suite_name in ["code-review", "spec-review"] {
        let destination = root.join(suite_name);
        if destination.try_exists()? {
            suite::Manifest::load(&destination)?;
        }
        for entry in fs::read_dir(staging.path().join(suite_name).join("tasks"))? {
            let target = destination.join("tasks").join(entry?.file_name());
            if target.try_exists()? {
                Task::load(&target)?;
            }
        }
    }
    fs::create_dir_all(&root)?;
    let mut result = Imported::default();
    for suite_name in ["code-review", "spec-review"] {
        let staged = staging.path().join(suite_name);
        let destination = root.join(suite_name);
        if !destination.try_exists()? {
            result.imported += fs::read_dir(staged.join("tasks"))?.count();
            fs::rename(staged, destination)?;
            continue;
        }
        fs::create_dir_all(destination.join("tasks"))?;
        for entry in fs::read_dir(staged.join("tasks"))? {
            let entry = entry?;
            let target = destination.join("tasks").join(entry.file_name());
            if target.try_exists()? {
                complete_trial_inputs(&target, &entry.path())?;
                result.skipped += 1;
            } else {
                fs::rename(entry.path(), target)?;
                result.imported += 1;
            }
        }
    }
    Ok(result)
}

/// Older imports predate Trial inputs. Add only the missing table,
/// preserving local criteria, formatting and any caller-supplied Trial inputs.
fn complete_trial_inputs(target: &Path, staged: &Path) -> Result<()> {
    let existing = Task::load(target)?;
    if existing.trial.is_some() {
        return Ok(());
    }
    let Some(inputs) = Task::load(staged)?.trial else {
        return Ok(());
    };
    let path = target.join("task.toml");
    let mut contents = fs::read_to_string(&path)?;
    contents.push('\n');
    contents.push_str(&toml::to_string(&BTreeMap::from([("trial", inputs)]))?);
    let temporary = tempfile::NamedTempFile::new_in(target)?;
    fs::write(temporary.path(), contents)?;
    temporary
        .persist(path)
        .context("could not complete Trial inputs")?;
    Ok(())
}

fn bug_task(
    home: &Path,
    dated: &Path,
    staging: &Path,
    grades: &Grades,
    id: &BugId,
    pr: PullRequest,
) -> Result<()> {
    let key = id.key();
    let criteria = grades
        .bugs
        .get(&key)
        .with_context(|| format!("missing catch criterion for bug {key}"))?;
    let name = format!(
        "bug-{}-pr-{}",
        key.strip_prefix('#').unwrap_or(&key),
        pr.number
    );
    write_task(
        staging,
        "code-review",
        &name,
        Task {
            format: 1,
            state: State::Approved,
            session: Session::CodeReview,
            repository: repository(home, "thirdshift")?,
            base: pr.merge_base,
            change: pr.head_as_merged,
            criteria: format!("{criteria}\nReport no wrong findings."),
            checks: Vec::new(),
            inputs: inputs(),
            // These replay bugs all belong to this project. Code review is
            // read-only: its branch names are local and its PR URL is unused.
            trial: Some(crate::trial::Inputs {
                issue_url: format!(
                    "{}/issues/{}",
                    env!("CARGO_PKG_REPOSITORY"),
                    pr.implements_issue
                ),
                base: "main".into(),
                branch: format!("issue-{}", pr.implements_issue),
                pr_url: format!("{}/pull/{}", env!("CARGO_PKG_REPOSITORY"), pr.number),
            }),
        },
        &context(dated, "thirdshift", pr.implements_issue)?,
    )?;
    if criteria.trim().is_empty() {
        bail!("blank catch criterion for bug {key}");
    }
    Ok(())
}

fn inputs() -> BTreeMap<String, String> {
    BTreeMap::from([(".thirdshift-issue.md".into(), "spec.md".into())])
}

fn repository(home: &Path, repo: &str) -> Result<String> {
    home.join("repos")
        .join(repo)
        .canonicalize()
        .with_context(|| format!("missing local replay repository {repo}"))?
        .into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("replay repository path is not UTF-8"))
}

fn write_task(staging: &Path, suite: &str, name: &str, task: Task, context: &str) -> Result<()> {
    format::name(name)?;
    let directory = staging.join(suite).join("tasks").join(name);
    // Duplicate source identities are an error, rather than silently dropping
    // a second criterion or overwriting the first.
    fs::create_dir(&directory).with_context(|| format!("duplicate Eval task {suite}/{name}"))?;
    fs::write(directory.join("task.toml"), toml::to_string(&task)?)?;
    fs::write(directory.join("spec.md"), context)?;
    Task::load(&directory)?;
    Ok(())
}

fn context(dated: &Path, repo: &str, number: u64) -> Result<String> {
    let directory = dated.join("context").join(repo);
    let mut done = BTreeSet::new();
    let mut text = String::new();
    let mut add = |number, role| -> Result<Option<Issue>> {
        if !done.insert(number) {
            return Ok(None);
        }
        let issue: Issue = json(&directory.join(format!("{number}.json")))?;
        text.push_str(&format!(
            "# {role} #{number}: {}\n\n{}\n\n",
            issue.title,
            issue.body.trim()
        ));
        Ok(Some(issue))
    };
    if let Some(issue) = add(number, "Issue")? {
        if let Some(parent) = issue.parent {
            add(parent, "Parent Spec")?;
        }
        for ticket in issue.tickets {
            add(ticket, "Ticket")?;
        }
    }
    Ok(text)
}

fn json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("can't read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid replay data {}", path.display()))
}

pub(super) fn suite_manifest(trials: usize) -> String {
    format!(
        r#"format = 1
trials_per_arm = {trials}
[dependencies]
factory_skills = ["skills/thirdshift-code-review/", "skills/thirdshift-codebase-design/", "skills/thirdshift-tdd/"]
session_prompts = ["src/prompt.rs", "src/trial.rs", "src/session.rs", "src/session/"]
harness_defaults = ["src/harness.rs", "src/harness/", "src/config.rs"]
model_defaults = ["src/harness.rs", "src/harness/", "src/config.rs", "src/setup.rs"]
effort_defaults = ["src/harness.rs", "src/harness/", "src/config.rs", "src/setup.rs"]
"#
    )
}
