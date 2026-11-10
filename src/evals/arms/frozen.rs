//! Content-addressed Process inputs, copied once and checked at each Trial.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{FixedHarness, Requested, checked, fixed_harness};
use crate::harness::{Choice, ChosenBy, Harness};

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Process {
    pub name: String,
    pub directory: PathBuf,
    pub files: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Step {
    pub name: String,
    pub harness: FixedHarness,
    pub model: Option<String>,
    pub effort: Option<String>,
}

pub(super) fn pin(
    harness: Harness,
    pinned: &mut BTreeMap<String, FixedHarness>,
) -> Result<FixedHarness> {
    if let Some(fixed) = pinned.get(harness.name()) {
        return Ok(fixed.clone());
    }
    let fixed = fixed_harness(harness)?;
    pinned.insert(harness.name().into(), fixed.clone());
    Ok(fixed)
}

impl Process {
    pub(super) fn freeze(
        request: &Requested,
        directory: &Path,
        pinned: &mut BTreeMap<String, FixedHarness>,
        compiled: Option<&str>,
    ) -> Result<Option<Self>> {
        let Some(process) = &request.process else {
            return Ok(None);
        };
        let inputs = crate::processes::source::local_inputs(
            process,
            &Choice {
                harness: request.harness,
                model: request.model.clone(),
                effort: request.effort.clone(),
                chosen_by: ChosenBy::Command,
            },
            compiled,
        )?;
        let process_dir = directory.join(".thirdshift/processes");
        fs::create_dir_all(&process_dir)?;
        fs::write(
            process_dir.join(format!("{}.toml", inputs.name)),
            inputs.text,
        )?;
        for name in inputs.skills {
            crate::skills::freeze_home_skill(&name, &directory.join(".thirdshift/skills"))?;
        }
        let steps = inputs
            .steps
            .into_iter()
            .map(|(name, choice)| {
                Ok(Step {
                    name,
                    harness: pin(choice.harness, pinned)?,
                    model: choice.model,
                    effort: choice.effort,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Self {
            name: inputs.name,
            directory: directory.to_path_buf(),
            files: hashes(directory)?,
            steps,
        }))
    }

    /// The Built-in file belongs to the selected build, which can differ
    /// from the Experiment runner. Ask that executable for its exact bytes.
    pub(super) fn from_build(
        request: &Requested,
        binary: &Path,
        process: &mut Option<Self>,
        pinned: &mut BTreeMap<String, FixedHarness>,
    ) -> Result<()> {
        if request.process.as_deref() == Some("ticket-run") {
            let output = checked(
                std::process::Command::new(binary).args(["process", "show", "ticket-run"]),
                "Arm Built-in process",
            )?;
            let text = String::from_utf8(output.stdout)?;
            let directory = process
                .as_ref()
                .context("Built-in Process was not frozen")?
                .directory
                .clone();
            *process = Self::freeze(request, &directory, pinned, Some(&text))?;
        }
        Ok(())
    }

    pub fn verify(&self) -> Result<()> {
        let actual = hashes(&self.directory)?;
        for name in self.files.keys().chain(actual.keys()) {
            ensure!(
                self.files.get(name) == actual.get(name),
                "frozen Arm file changed: {name}"
            );
        }
        Ok(())
    }

    /// Copy only verified bytes, so an edit racing the launch cannot slip in.
    pub fn install(&self, home: &Path) -> Result<PathBuf> {
        self.verify()?;
        for (name, expected) in &self.files {
            crate::evals::format::relative_path(name)?;
            let bytes = fs::read(self.directory.join(name))
                .with_context(|| format!("frozen Arm file changed: {name}"))?;
            ensure!(
                digest(&bytes) == *expected,
                "frozen Arm file changed: {name}"
            );
            let destination = home.join(name);
            fs::create_dir_all(destination.parent().context("frozen file has no parent")?)?;
            let permissions = fs::metadata(self.directory.join(name))?.permissions();
            fs::write(&destination, bytes)?;
            fs::set_permissions(destination, permissions)?;
        }
        Ok(home
            .join(".thirdshift/processes")
            .join(format!("{}.toml", self.name)))
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hashes(directory: &Path) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    collect(directory, directory, 0, &mut hashes)?;
    Ok(hashes)
}

fn collect(
    root: &Path,
    directory: &Path,
    depth: usize,
    hashes: &mut BTreeMap<String, String>,
) -> Result<()> {
    ensure!(
        depth <= 64,
        "frozen Arm directory nested too deeply: {}",
        directory.display()
    );
    ensure!(
        fs::symlink_metadata(directory)?.file_type().is_dir(),
        "frozen Arm directory changed: {}",
        directory.display()
    );
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .strip_prefix(root)?
            .to_str()
            .context("frozen file path must be UTF-8")?;
        let kind = entry.file_type()?;
        ensure!(
            kind.is_file() || kind.is_dir(),
            "frozen Arm file changed: {name} (not a regular file or directory)"
        );
        if kind.is_file() {
            let bytes = fs::read(&path)
                .with_context(|| format!("could not check frozen Arm file {name}"))?;
            hashes.insert(name.into(), digest(&bytes));
        } else {
            collect(root, &path, depth + 1, hashes)?;
        }
    }
    Ok(())
}
