//! Known Launch directories, shared by CLI discovery and Console starts.
use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{config, cronjobs, git::Git, issue::Repo};

#[cfg(test)]
mod tests;

#[derive(Clone, Serialize)]
pub(crate) struct Directory {
    pub repository: String,
    pub path: PathBuf,
    #[serde(skip)]
    pub output_log: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
pub(crate) struct Inventory {
    pub directories: Vec<Directory>,
}

pub(crate) fn load() -> Result<Inventory> {
    let config = config::UserConfig::load()?;
    Ok(Captured::capture(&config)?.directories()?.clone())
}

/// Associated facts for one entry, independent of directory deduplication.
pub(crate) struct Entry {
    pub job: cronjobs::CronJob,
    pub launch: Option<cronjobs::LaunchFacts>,
    pub directory: Option<Directory>,
}

/// One owning-machine operation. Failed observations live as long as successes;
/// configured directories and Pass history are acquired only by their projections.
pub(crate) struct Captured {
    entries: Result<Vec<Entry>>,
    configured: Vec<PathBuf>,
    logs_dir: PathBuf,
    checkouts: RefCell<Checkouts>,
    directories: OnceCell<Result<Inventory>>,
    cronjobs: OnceCell<Result<cronjobs::CronJobs>>,
}

impl Captured {
    pub(crate) fn capture(config: &config::UserConfig) -> Result<Self> {
        let home = config::home()?;
        Ok(Self::new(config, &home, cronjobs::entries()))
    }

    fn new(
        config: &config::UserConfig,
        home: &Path,
        input: Result<Vec<cronjobs::InterpretedEntry>>,
    ) -> Self {
        let mut checkouts = Checkouts::default();
        let entries = input.map(|entries| {
            entries
                .into_iter()
                .map(|entry| {
                    let launch = entry.interpretation.into_launch(home);
                    let directory = launch.as_ref().and_then(|facts| {
                        let mut directory = checkouts.resolve(&facts.directory).ok()?;
                        directory.output_log = facts.output_log.clone();
                        Some(directory)
                    });
                    Entry {
                        job: entry.job,
                        launch,
                        directory,
                    }
                })
                .collect()
        });
        Self {
            entries,
            configured: config.launch_directories.clone(),
            logs_dir: config.logs_dir.clone(),
            checkouts: RefCell::new(checkouts),
            directories: OnceCell::new(),
            cronjobs: OnceCell::new(),
        }
    }

    /// Start/discovery projections tolerate only an absent crontab executable.
    pub(crate) fn entries(&self) -> Result<&[Entry]> {
        match &self.entries {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(&[])
            }
            result => Ok(observed(result)?.as_slice()),
        }
    }

    pub(crate) fn directories(&self) -> Result<&Inventory> {
        observed(self.directories.get_or_init(|| {
            let mut directories = BTreeMap::new();
            for directory in self
                .entries()?
                .iter()
                .filter_map(|entry| entry.directory.as_ref())
            {
                directories
                    .entry(directory.path.clone())
                    .or_insert_with(|| directory.clone());
            }
            for path in &self.configured {
                // Validate aliases too; a Cron entry never authorizes a configured path.
                let directory = self.checkouts.borrow_mut().resolve(path).with_context(|| {
                    format!("invalid configured Launch directory {}", path.display())
                })?;
                directories
                    .entry(directory.path.clone())
                    .or_insert(directory);
            }
            Ok(Inventory {
                directories: directories.into_values().collect(),
            })
        }))
    }

    /// Strict inventory, without configured-directory validation or start data.
    pub(crate) fn cronjobs(&self) -> Result<&cronjobs::CronJobs> {
        observed(self.cronjobs.get_or_init(|| {
            let jobs = observed(&self.entries)?
                .iter()
                .map(|entry| {
                    let mut job = entry.job.clone();
                    job.repository = entry
                        .directory
                        .as_ref()
                        .map(|directory| directory.repository.clone());
                    job
                })
                .collect();
            cronjobs::inventory(jobs, &self.logs_dir)
        }))
    }
}

fn observed<T>(result: &Result<T>) -> Result<&T> {
    result
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error:#}"))
}

#[derive(Default)]
struct Checkouts {
    canonical: BTreeMap<PathBuf, Result<PathBuf>>,
    origins: BTreeMap<PathBuf, Result<String>>,
}

impl Checkouts {
    fn resolve(&mut self, literal: &Path) -> Result<Directory> {
        let path = observed(
            self.canonical
                .entry(literal.to_owned())
                .or_insert_with(|| Ok(literal.canonicalize()?)),
        )?;
        let repository = observed(self.origins.entry(path.clone()).or_insert_with(|| {
            let origin = Git::new(path).run(&["config", "--get", "remote.origin.url"])?;
            Ok(Repo::of_origin(&origin)
                .context("Launch directory origin is not on GitHub")?
                .slug())
        }))?;
        Ok(Directory {
            repository: repository.clone(),
            path: path.clone(),
            output_log: None,
        })
    }
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
