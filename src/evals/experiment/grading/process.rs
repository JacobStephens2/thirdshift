//! Grade committed process work with tests from the private reference fix.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use super::Task;
use crate::evals::{format, repository};
use crate::sandbox::code_grader::{self, Check};
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
    let snapshot = match snapshot(task, trial) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Ok(vec![Check {
                command: None,
                passed: false,
                reason: format!("could not prepare process Grader: {error:#}"),
            }]);
        }
    };
    let commands: Vec<_> = grading
        .fail_to_pass
        .iter()
        .chain(&grading.pass_to_pass)
        .cloned()
        .collect();
    let mut checks = code_grader::execute(
        &snapshot.path().join("repository"),
        &Reports {
            standards: None,
            spec: None,
        },
        &commands,
        timeout,
        caps,
    )?;
    for check in &mut checks {
        if let Some(command) = &check.command {
            check.reason = format!(
                "{}: {}",
                crate::args::shell_command(command.clone()),
                check.reason
            );
        }
    }
    Ok(checks)
}

fn snapshot(task: &Task, trial: &Path) -> Result<tempfile::TempDir> {
    let grading = task
        .manifest
        .process_grading
        .as_ref()
        .context("missing process_grading")?;
    let snapshot = tempfile::Builder::new()
        .prefix("process-grader-")
        .tempdir()?;
    let destination = snapshot.path().join("repository");
    fs::create_dir(&destination)?;
    let commit = repository::git(trial, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    for file in files(trial, &commit)? {
        file.write(trial, &destination)?;
    }
    let source = task.directory.join(&task.manifest.repository);
    let reference = files(&source, &task.manifest.change)?;
    for path in grading.test_files.iter().chain(&grading.test_fakes) {
        let file = reference
            .iter()
            .find(|file| &file.path == path)
            .with_context(|| format!("reference fix has no test file {path}"))?;
        ensure!(
            matches!(file.mode.as_str(), "100644" | "100755"),
            "reference test must be a regular file: {path}"
        );
        file.write(&source, &destination)?;
    }
    Ok(snapshot)
}

struct File {
    path: String,
    mode: String,
    object: String,
}

fn files(repository: &Path, commit: &str) -> Result<Vec<File>> {
    let tree = repository::bytes(repository, &["ls-tree", "-rz", "--full-tree", commit], None)?;
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
    fn write(&self, repository: &Path, destination: &Path) -> Result<()> {
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
        let contents = repository::bytes(repository, &["cat-file", "blob", &self.object], None)?;
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
