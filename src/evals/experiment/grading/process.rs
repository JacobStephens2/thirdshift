//! Grade committed process work with tests from the private reference fix.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use super::Task;
use crate::evals::{format, repository};
use crate::sandbox::code_grader::{self, Check, CheckRun};
use crate::scope::Caps;
use crate::trial::Reports;

pub(super) fn execute(
    task: &Task,
    trial: &Path,
    timeout: Duration,
    caps: &Caps,
) -> Result<Vec<Check>> {
    let grading = task
        .manifest
        .process_grading
        .as_ref()
        .context("missing process_grading")?;
    let deadline = Instant::now() + timeout;
    let commands: Vec<_> = grading
        .fail_to_pass
        .iter()
        .chain(&grading.pass_to_pass)
        .cloned()
        .collect();
    let source = task.directory.join(&task.manifest.repository);
    let paths: Vec<_> = grading
        .test_files
        .iter()
        .chain(&grading.test_fakes)
        .cloned()
        .collect();
    let code = match head(trial, deadline) {
        Ok(commit) => commit,
        Err(error) => return Ok(vec![unprepared("process Grader", &error).check]),
    };
    let ran = overlaid(
        Source {
            repository: trial,
            commit: &code,
        },
        Source {
            repository: &source,
            commit: &task.manifest.change,
        },
        &paths,
        &commands,
        GraderRun {
            name: "process Grader",
            deadline,
            caps,
        },
    )?;
    Ok(ran.into_iter().map(|ran| ran.check).collect())
}

/// The commit checked out in `repository`.
pub(in super::super) fn head(repository: &Path, deadline: Instant) -> Result<String> {
    let commit = repository::bytes_until(
        repository,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        deadline,
    )?;
    Ok(std::str::from_utf8(&commit)?.trim().to_owned())
}

/// One commit in one repository.
pub(in super::super) struct Source<'a> {
    pub repository: &'a Path,
    pub commit: &'a str,
}

/// One named code-Grader run, bounded by a deadline shared with its snapshot.
pub(in super::super) struct GraderRun<'a> {
    pub name: &'a str,
    pub deadline: Instant,
    pub caps: &'a Caps,
}

/// Export `code`'s commit, lay `paths` from `tests`'s commit over it, and run
/// `commands` offline in the code-Grader sandbox, all before the deadline.
/// A snapshot that cannot be prepared is one failed check that never exited.
pub(in super::super) fn overlaid(
    code: Source<'_>,
    tests: Source<'_>,
    paths: &[String],
    commands: &[Vec<String>],
    run: GraderRun<'_>,
) -> Result<Vec<CheckRun>> {
    let GraderRun {
        name,
        deadline,
        caps,
    } = run;
    let snapshot = match snapshot(&code, &tests, paths, deadline) {
        Ok(snapshot) => snapshot,
        Err(error) => return Ok(vec![unprepared(name, &error)]),
    };
    let mut ran = code_grader::execute_runs(
        &snapshot.path().join("repository"),
        &Reports {
            standards: None,
            spec: None,
        },
        commands,
        code_grader::Timeout::Deadline(deadline),
        caps,
        true,
    )?;
    for CheckRun { check, .. } in &mut ran {
        if let Some(command) = &check.command {
            check.reason = format!(
                "{}: {}",
                crate::args::shell_command(command.clone()),
                check.reason
            );
        }
    }
    Ok(ran)
}

fn unprepared(what: &str, error: &anyhow::Error) -> CheckRun {
    CheckRun {
        check: Check {
            command: None,
            passed: false,
            reason: format!("could not prepare {what}: {error:#}"),
        },
        exited: false,
    }
}

/// Files a commit adds or changes since `base`, in path order.
pub(in super::super) fn changed(
    repository: &Path,
    base: &str,
    commit: &str,
    deadline: Instant,
) -> Result<Vec<String>> {
    let names = repository::bytes_until(
        repository,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=AM",
            base,
            commit,
            "--",
        ],
        deadline,
    )?;
    names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| {
            let name = std::str::from_utf8(name).context("changed paths must be UTF-8")?;
            format::relative_path(name)?;
            Ok(name.to_owned())
        })
        .collect()
}

fn snapshot(
    code: &Source<'_>,
    tests: &Source<'_>,
    paths: &[String],
    deadline: Instant,
) -> Result<tempfile::TempDir> {
    let snapshot = tempfile::Builder::new()
        .prefix("process-grader-")
        .tempdir()?;
    let destination = snapshot.path().join("repository");
    fs::create_dir(&destination)?;
    for file in files(code.repository, code.commit, deadline)? {
        file.write(code.repository, &destination, deadline)?;
    }
    let overlay = files(tests.repository, tests.commit, deadline)?;
    for path in paths {
        let file = overlay
            .iter()
            .find(|file| &file.path == path)
            .with_context(|| format!("test commit has no test file {path}"))?;
        ensure!(
            matches!(file.mode.as_str(), "100644" | "100755"),
            "overlaid test must be a regular file: {path}"
        );
        file.write(tests.repository, &destination, deadline)?;
    }
    Ok(snapshot)
}

struct File {
    path: String,
    mode: String,
    object: String,
}

fn files(repository: &Path, commit: &str, deadline: Instant) -> Result<Vec<File>> {
    let tree = repository::bytes_until(
        repository,
        &["ls-tree", "-rz", "--full-tree", commit],
        deadline,
    )?;
    tree.split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = std::str::from_utf8(entry).context("process Grader paths must be UTF-8")?;
            let (metadata, path) = entry.split_once('\t').context("invalid Git tree entry")?;
            format::relative_path(path)?;
            let mut fields = metadata.split_whitespace();
            let mode = fields.next().context("missing Git mode")?.to_owned();
            ensure!(
                fields.next() == Some("blob"),
                "process Grader cannot export non-file {path}"
            );
            let object = fields.next().context("missing Git object")?.to_owned();
            Ok(File {
                path: path.into(),
                mode,
                object,
            })
        })
        .collect()
}

impl File {
    fn write(&self, repository: &Path, destination: &Path, deadline: Instant) -> Result<()> {
        // Replace Trial symlinks and directories rather than following them
        // while laying trusted tests over the exported commit.
        let mut target = destination.to_path_buf();
        let parts: Vec<_> = self.path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            target.push(part);
            if !fs::symlink_metadata(&target).is_ok_and(|kind| kind.is_dir()) {
                remove(&target)?;
                fs::create_dir(&target)?;
            }
        }
        target.push(parts[parts.len() - 1]);
        remove(&target)?;
        let contents =
            repository::bytes_until(repository, &["cat-file", "blob", &self.object], deadline)?;
        match self.mode.as_str() {
            "100644" | "100755" => {
                fs::write(&target, contents)?;
                fs::set_permissions(
                    &target,
                    fs::Permissions::from_mode(if self.mode == "100755" { 0o755 } else { 0o644 }),
                )?;
            }
            "120000" => symlink(std::ffi::OsStr::from_bytes(&contents), target)?,
            mode => bail!("unsupported Git mode {mode}"),
        }
        Ok(())
    }
}

fn remove(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(kind) if kind.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
