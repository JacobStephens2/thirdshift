//! Prepare and screen real process tasks without exposing their reference fix
//! to a Trial. Screening is a host operation and may use the network.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::format::{self, Task};
use super::repository::{bytes, git};
use crate::process::{self, Control, Interruption};

mod layout;
mod proposals;
pub(super) use proposals::propose;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TestSelection {
    pub overlay_files: Vec<String>,
    pub shared_fakes: Vec<String>,
    pub fail_to_pass: Vec<Vec<String>>,
    #[serde(default)]
    pub pass_to_pass: Vec<Vec<String>>,
}

impl TestSelection {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.overlay_files.is_empty(),
            "process task needs test overlay files"
        );
        ensure!(
            !self.fail_to_pass.is_empty(),
            "process task needs fail-to-pass selectors"
        );
        let mut seen = BTreeSet::new();
        for path in self.overlay_files.iter().chain(&self.shared_fakes) {
            format::relative_path(path)?;
            ensure!(
                is_test_path(path),
                "process overlay is not a separate test file: {path}"
            );
            ensure!(seen.insert(path), "duplicate process overlay: {path}");
        }
        for selector in self.fail_to_pass.iter().chain(&self.pass_to_pass) {
            ensure!(
                selector.first().is_some_and(|s| !s.trim().is_empty()),
                "process selectors must be nonempty argument vectors"
            );
        }
        Ok(())
    }
}

fn is_test_path(path: &str) -> bool {
    path.split('/')
        .any(|part| matches!(part, "tests" | "test" | "__tests__"))
        || Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with("test_")
                    || [
                        "_test.rs", ".test.js", ".test.ts", ".spec.js", ".spec.ts", "_test.py",
                    ]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            })
}

pub(super) fn screen(source: &Path, task: &Task) -> Result<()> {
    let selection = task
        .process_tests
        .as_ref()
        .context("process task needs process_tests for preparation")?;
    selection.validate()?;
    layout::reject_inline_changes(source, &task.base, &task.change)?;
    let temporary = tempfile::Builder::new()
        .prefix("process-screen-")
        .tempdir()?;
    let object_format = git(source, &["rev-parse", "--show-object-format"])?;
    for (label, commit) in [("base", &task.base), ("merge", &task.change)] {
        let checkout = temporary.path().join(label);
        fs::create_dir(&checkout)?;
        git(
            &checkout,
            &[
                "init",
                "-q",
                "--template=",
                &format!("--object-format={object_format}"),
            ],
        )?;
        git(
            &checkout,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                source.to_str().context("non-UTF-8 repository")?,
                commit,
            ],
        )?;
        git(&checkout, &["checkout", "-q", "--detach", commit])?;
        overlay(source, &task.change, selection, &checkout)?;
        for selector in &selection.fail_to_pass {
            let output = process::output_until(
                Command::new(&selector[0])
                    .args(&selector[1..])
                    .current_dir(&checkout),
                None,
                Control {
                    name: "process task screening",
                    interruption: Interruption::Ordinary,
                    stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
                },
                Instant::now() + Duration::from_secs(600),
                &|| Ok(()),
            )
            .with_context(|| format!("could not screen selector {selector:?} at {label}"))?;
            if label == "base" {
                ensure!(
                    !output.status.success(),
                    "fail-to-pass selector {selector:?} passes at base"
                );
            } else {
                ensure!(
                    output.status.success(),
                    "fail-to-pass selector {selector:?} fails at merge:\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    Ok(())
}

fn overlay(source: &Path, commit: &str, selection: &TestSelection, checkout: &Path) -> Result<()> {
    for path in selection
        .overlay_files
        .iter()
        .chain(&selection.shared_fakes)
    {
        let entry = git(source, &["ls-tree", commit, "--", path])?;
        ensure!(
            entry.starts_with("100644 blob ") || entry.starts_with("100755 blob "),
            "process overlay must be a regular file at merge: {path}"
        );
        let contents = bytes(source, &["show", &format!("{commit}:{path}")], None)?;
        let mut target = checkout.to_path_buf();
        for part in path.split('/') {
            target.push(part);
            ensure!(
                !fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()),
                "process overlay crosses a symlink: {path}"
            );
        }
        fs::create_dir_all(target.parent().context("overlay has no parent")?)?;
        fs::write(target, contents)?;
    }
    Ok(())
}
