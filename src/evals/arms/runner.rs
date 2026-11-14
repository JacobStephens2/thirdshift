//! A Custom runner's declared inputs, frozen independently of a thirdshift build.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::frozen::{digest, hashes};
use crate::git::Git;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub command: Vec<String>,
    #[serde(default)]
    pub files: BTreeMap<String, Target>,
    #[serde(default)]
    pub roles: BTreeMap<String, Role>,
    #[serde(default)]
    pub git_exclude: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum Target {
    Bundle(String),
    Home { home: String },
}

impl Target {
    fn path(&self) -> Result<PathBuf> {
        let (root, relative) = match self {
            Self::Bundle(relative) => ("bundle", relative),
            Self::Home { home } => ("home", home),
        };
        crate::evals::format::relative_path(relative)?;
        Ok(Path::new(root).join(relative))
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Role {
    pub model: String,
    pub effort: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Provenance {
    pub repository: PathBuf,
    pub origin: Option<String>,
    pub path: PathBuf,
    pub commit: String,
    pub dirty: bool,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Runner {
    pub name: String,
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub files: BTreeMap<String, String>,
    pub provenance: BTreeMap<String, Provenance>,
}

impl Runner {
    pub fn freeze(name: &str, directory: &Path) -> Result<Self> {
        let path = if name.ends_with(".toml") {
            PathBuf::from(name)
        } else {
            crate::evals::format::name(name)?;
            crate::config::home()?
                .join(".thirdshift/runners")
                .join(format!("{name}.toml"))
        }
        .canonicalize()
        .with_context(|| format!("cannot resolve Custom runner {name}"))?;
        let bytes = fs::read(&path)?;
        let manifest: Manifest = toml::from_str(std::str::from_utf8(&bytes)?)?;
        ensure!(
            !manifest.command.is_empty() && !manifest.command[0].is_empty(),
            "Custom runner command must contain a program"
        );
        ensure!(
            manifest.command.iter().all(|arg| !arg.contains('\0')),
            "Custom runner command contains NUL"
        );
        ensure!(
            manifest
                .git_exclude
                .iter()
                .all(|pattern| !pattern.contains(['\n', '\r', '\0'])),
            "git_exclude patterns must each be one line"
        );
        fs::create_dir_all(directory.join("bundle"))?;
        fs::create_dir_all(directory.join("home"))?;
        fs::write(directory.join("manifest.toml"), &bytes)?;
        let mut provenance = BTreeMap::new();
        if let Some(origin) = source_provenance(&path)? {
            provenance.insert("manifest.toml".into(), origin);
        }
        for (source, target) in &manifest.files {
            let source_path = path
                .parent()
                .context("runner manifest has no parent")?
                .join(source)
                .canonicalize()?;
            let destination = directory.join(target.path()?);
            copy(&source_path, &destination, 0)?;
            if let Some(origin) = source_provenance(&source_path)? {
                provenance.insert(source.clone(), origin);
            }
        }
        Ok(Self {
            name: name.into(),
            directory: directory.to_path_buf(),
            manifest,
            files: hashes(directory)?,
            provenance,
        })
    }

    pub fn source_digest(name: &str) -> Result<String> {
        let temporary = tempfile::tempdir()?;
        Self::freeze(name, temporary.path())?.digest()
    }

    pub fn digest(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&self.files)?))
    }

    pub fn verify(&self) -> Result<()> {
        ensure!(
            hashes(&self.directory)? == self.files,
            "frozen Custom runner file changed"
        );
        let frozen: Manifest =
            toml::from_str(&fs::read_to_string(self.directory.join("manifest.toml"))?)?;
        ensure!(
            serde_json::to_vec(&frozen)? == serde_json::to_vec(&self.manifest)?,
            "frozen Custom runner manifest changed"
        );
        Ok(())
    }

    /// Install checked bytes into a fresh attempt, preserving executable modes.
    pub fn install(&self, attempt: &Path) -> Result<PathBuf> {
        self.verify()?;
        let runner_dir = attempt.join("runner");
        fs::create_dir_all(&runner_dir)?;
        fs::create_dir_all(attempt.join("home"))?;
        for (name, expected) in &self.files {
            let target = if let Some(relative) = name.strip_prefix("bundle/") {
                runner_dir.join(relative)
            } else if name.starts_with("home/") {
                attempt.join(name)
            } else {
                continue;
            };
            let source = self.directory.join(name);
            let bytes = fs::read(&source)?;
            ensure!(
                digest(&bytes) == *expected,
                "frozen Custom runner file changed: {name}"
            );
            fs::create_dir_all(target.parent().context("runner file has no parent")?)?;
            fs::write(&target, bytes)?;
            fs::set_permissions(target, fs::metadata(source)?.permissions())?;
        }
        Ok(runner_dir)
    }
}

fn source_provenance(path: &Path) -> Result<Option<Provenance>> {
    let directory = if path.is_dir() {
        path
    } else {
        path.parent().context("source has no parent")?
    };
    let git = Git::new(directory);
    let Ok(root) = git.run(&["rev-parse", "--show-toplevel"]) else {
        return Ok(None);
    };
    let repository = PathBuf::from(root).canonicalize()?;
    Ok(Some(Provenance {
        path: path.strip_prefix(&repository)?.to_path_buf(),
        commit: git.run(&["rev-parse", "HEAD"])?,
        origin: git.run(&["config", "--get", "remote.origin.url"]).ok(),
        dirty: !git
            .run(&["status", "--porcelain", "--untracked-files=all"])?
            .is_empty(),
        repository,
    }))
}

fn copy(source: &Path, destination: &Path, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "Custom runner directory nested too deeply");
    let kind = fs::symlink_metadata(source)?.file_type();
    ensure!(
        kind.is_dir() || kind.is_file(),
        "Custom runner inputs must be regular files or directories: {}",
        source.display()
    );
    ensure!(
        !destination.exists(),
        "overlapping Custom runner file targets: {}",
        destination.display()
    );
    if kind.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy(
                &entry.path(),
                &destination.join(entry.file_name()),
                depth + 1,
            )?;
        }
    } else {
        fs::create_dir_all(destination.parent().context("runner file has no parent")?)?;
        fs::copy(source, destination)?;
    }
    Ok(())
}
