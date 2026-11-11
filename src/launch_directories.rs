//! Known Launch directories, shared by CLI discovery and Console starts.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{config, cronjobs, git::Git, issue::Repo};

#[derive(Serialize)]
pub(crate) struct Directory {
    pub repository: String,
    pub path: PathBuf,
    #[serde(skip)]
    pub output_log: Option<PathBuf>,
}

#[derive(Serialize)]
pub(crate) struct Inventory {
    pub directories: Vec<Directory>,
}

pub(crate) fn load() -> Result<Inventory> {
    let home = config::home()?;
    let config = config::UserConfig::load()?;
    let jobs = match cronjobs::jobs() {
        Ok(jobs) => jobs,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    let mut directories = BTreeMap::new();
    for job in jobs {
        if let Some(directory) = cronjobs::Interpretation::new(&job.command)
            .into_launch(&home)
            .and_then(from_facts)
        {
            directories
                .entry(directory.path.clone())
                .or_insert(directory);
        }
    }
    for path in config.launch_directories {
        let directory = resolve(&path)
            .with_context(|| format!("invalid configured Launch directory {}", path.display()))?;
        directories
            .entry(directory.path.clone())
            .or_insert(directory);
    }
    Ok(Inventory {
        directories: directories.into_values().collect(),
    })
}

fn resolve(path: &Path) -> Result<Directory> {
    let path = path.canonicalize()?;
    let origin = Git::new(&path).run(&["config", "--get", "remote.origin.url"])?;
    let repo = Repo::of_origin(&origin).context("Launch directory origin is not on GitHub")?;
    Ok(Directory {
        repository: repo.slug(),
        path,
        output_log: None,
    })
}

/// Resolve literal facts supplied by Cron job interpretation; never run Cron text.
pub(crate) fn from_facts(facts: cronjobs::LaunchFacts) -> Option<Directory> {
    let mut directory = resolve(&facts.directory).ok()?;
    directory.output_log = facts.output_log;
    Some(directory)
}

pub fn show(json: bool) -> Result<String> {
    let inventory = load()?;
    if json {
        return Ok(serde_json::to_string_pretty(&inventory)?);
    }
    Ok(inventory
        .directories
        .iter()
        .map(|directory| format!("{}  {}", directory.repository, directory.path.display()))
        .collect::<Vec<_>>()
        .join("\n"))
}
