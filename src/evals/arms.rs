//! Prepare both Arms before exposing their fixed binaries to the Trial runner.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::format;
use crate::harness::Harness;
use crate::process::{self, Control, Interruption};

mod frozen;
pub(crate) use frozen::Process;
mod runner;
pub(crate) use runner::Runner;

pub(crate) struct Requested {
    commit: String,
    harness: Harness,
    process: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    runner: Option<String>,
}

impl Requested {
    pub fn new(commit: &str, harness: &str) -> Result<Self> {
        format::commit_id(commit)?;
        Ok(Self {
            commit: commit.into(),
            harness: Harness::named(harness).context("unknown Arm Harness")?,
            process: None,
            model: None,
            effort: None,
            runner: None,
        })
    }

    pub fn runner(name: &str) -> Self {
        Self {
            commit: String::new(),
            harness: Harness::Claude,
            process: None,
            model: None,
            effort: None,
            runner: Some(name.into()),
        }
    }

    pub fn with_process(
        mut self,
        process: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Self {
        self.process = process.map(String::from);
        self.model = model.map(String::from);
        self.effort = effort.map(String::from);
        self
    }

    pub fn options(arms: &mut [Self; 2], options: &[String]) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        let mut options = options.iter();
        while let Some(flag) = options.next() {
            anyhow::ensure!(seen.insert(flag), "repeated {flag}");
            let value = options
                .next()
                .with_context(|| format!("{flag} requires a value"))?;
            let (index, field) = if let Some(field) = flag.strip_prefix("--baseline-") {
                (0, field)
            } else if let Some(field) = flag.strip_prefix("--candidate-") {
                (1, field)
            } else {
                bail!("unknown Arm flag: {flag}");
            };
            match field {
                "process" => arms[index].process = Some(value.clone()),
                "model" => arms[index].model = Some(value.clone()),
                "effort" => arms[index].effort = Some(value.clone()),
                _ => bail!("unknown Arm flag: {flag}"),
            }
        }
        if arms.iter().any(|arm| arm.process.is_some()) {
            for arm in arms {
                arm.process.get_or_insert_with(|| "ticket-run".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct FixedHarness {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Arm {
    pub commit: Option<String>,
    pub binary: PathBuf,
    pub harness: Option<FixedHarness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<Process>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<Runner>,
    /// Provider identities fixed before Trials; absent in older Arm records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub families: Option<BTreeSet<String>>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Prepared {
    format: u32,
    pub baseline: Arm,
    pub candidate: Arm,
}

pub(super) fn prepare(repository: &Path, arms: [Requested; 2], record: &Path) -> Result<Prepared> {
    if record.try_exists()? {
        bail!("Arm record already exists: {}", record.display());
    }
    // Resolve all Harnesses at the start, before either potentially long build.
    // The same Harness is resolved only once even when both Arms use it.
    let parent = record
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let frozen = tempfile::Builder::new()
        .prefix(".arm-inputs-")
        .tempdir_in(parent)?;
    let frozen_root = frozen.path().canonicalize()?;
    let mut pinned = std::collections::BTreeMap::new();
    let mut inputs = Vec::new();
    for (name, request) in ["baseline", "candidate"].into_iter().zip(&arms) {
        let directory = frozen_root.join(name);
        inputs.push(if let Some(runner) = &request.runner {
            Arm {
                commit: None,
                binary: PathBuf::new(),
                harness: None,
                process: None,
                runner: Some(Runner::freeze(runner, &directory)?),
                families: None,
            }
        } else {
            Arm {
                commit: Some(request.commit.clone()),
                binary: PathBuf::new(),
                harness: Some(frozen::pin(request.harness, &mut pinned)?),
                process: Process::freeze(request, &directory, &mut pinned, None)?,
                runner: None,
                families: None,
            }
        });
    }
    for (request, arm) in arms.iter().zip(&mut inputs) {
        if request.runner.is_none() {
            let builds = crate::config::home()?.join(".thirdshift/evals/builds");
            fs::create_dir_all(&builds)?;
            arm.binary = build(repository, &builds.canonicalize()?, &request.commit)?;
            Process::from_build(request, &arm.binary, &mut arm.process, &mut pinned)?;
            let mut families = BTreeSet::new();
            let models = std::iter::once((request.harness, request.model.as_deref())).chain(
                arm.process
                    .iter()
                    .flat_map(|process| &process.steps)
                    .map(|step| {
                        (
                            Harness::named(&step.harness.name).expect("frozen Harness"),
                            step.model.as_deref(),
                        )
                    }),
            );
            for (harness, model) in models {
                let hosts = if harness == Harness::Kimi && model.is_some() {
                    crate::proxy::allowed_hosts(harness, model)?
                } else {
                    Vec::new()
                };
                families.insert(crate::proxy::harness_family(harness, model, &hosts));
            }
            arm.families = Some(families);
        }
    }
    let [baseline, candidate]: [Arm; 2] = inputs.try_into().ok().expect("two Arms");
    let prepared = Prepared {
        format: 1,
        baseline,
        candidate,
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
    if [&prepared.baseline, &prepared.candidate]
        .iter()
        .any(|arm| arm.process.is_some() || arm.runner.is_some())
    {
        let _ = frozen.keep();
    }
    Ok(prepared)
}

pub(super) fn fixed_harness(harness: Harness) -> Result<FixedHarness> {
    let name = harness.name();
    let path = process::find_executable(name)
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

pub(super) fn build(repository: &Path, builds: &Path, commit: &str) -> Result<PathBuf> {
    let git = crate::git::Git::new(repository);
    if git.run(&["--no-replace-objects", "cat-file", "-t", commit])? != "commit" {
        bail!("Arm {commit} is not a commit");
    }
    let lock = fs::File::create(builds.join(format!("{commit}.lock")))?;
    lock.lock()?;
    let cached = builds.join(commit);
    let binary = cached.join("thirdshift");
    if process::is_executable(&binary) {
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
    if !process::is_executable(&built) {
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
