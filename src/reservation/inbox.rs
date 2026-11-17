//! Retained observations of reservations that actually blocked work. Console
//! refreshes only read origin; recovery belongs to scheduled Pass startup.
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::recovery::{Observation, inventory};
use crate::{config::UserConfig, git::Git, issue::Repo};

#[derive(Default, Deserialize, Serialize)]
struct File {
    repository: String,
    launch: PathBuf,
    observations: Vec<Observation>,
}

fn read(path: &Path) -> Result<File> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(File::default()),
        Err(error) => Err(error.into()),
    }
}

fn save(path: &Path, file: &File) -> Result<()> {
    let bytes = serde_json::to_vec(file)?;
    if std::fs::read(path).ok().as_deref() == Some(&bytes) {
        return Ok(());
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(path.parent().context("no reservation directory")?)?;
    staged.write_all(&bytes)?;
    staged.persist(path)?;
    Ok(())
}

fn locked(root: &Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(root)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".reservations.lock"))?;
    file.lock()?;
    Ok(file)
}

pub(super) fn reconcile(git: &Git, repo: &Repo, observed: &[(String, String)]) -> Result<()> {
    let root = crate::logs::root(repo);
    let _lock = locked(&root)?;
    let path = root.join("reservations.json");
    let mut file = read(&path)?;
    file.observations.retain(|row| {
        observed
            .iter()
            .any(|(reference, sha)| reference == &row.reference && sha == &row.sha)
    });
    file.repository = repo.slug();
    file.launch = git.dir().into();
    save(&path, &file)
}

pub(super) fn blocked(git: &Git, repo: &Repo, observation: Observation) -> Result<()> {
    if observation.owner.testable(crate::host::name().as_deref()) {
        return Ok(());
    }
    let root = crate::logs::root(repo);
    let _lock = locked(&root)?;
    let path = root.join("reservations.json");
    let mut file = read(&path)?;
    file.repository = repo.slug();
    // Release takes its reservation in an isolated temporary repository. Pass
    // startup already retained the durable Launch directory for later reads.
    if file.launch.as_os_str().is_empty() {
        file.launch = git.dir().into();
    }
    file.observations
        .retain(|row| row.reference != observation.reference);
    file.observations.push(observation);
    file.observations
        .sort_by(|a, b| a.reference.cmp(&b.reference));
    save(&path, &file)
}

fn files(config: &UserConfig) -> Vec<PathBuf> {
    let directories = |path: &Path| {
        std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect::<Vec<_>>()
    };
    directories(&config.logs_dir)
        .into_iter()
        .flat_map(|owner| directories(&owner))
        .map(|repo| repo.join("reservations.json"))
        .filter(|path| path.is_file())
        .collect()
}

/// Refresh known blocking observations. An unavailable origin keeps the last
/// warning visible; neither Console nor status deletes remote refs.
pub(crate) fn refresh(config: &UserConfig) {
    for path in files(config) {
        let result = (|| -> Result<()> {
            let file = read(&path)?;
            if file.observations.is_empty() {
                return Ok(());
            }
            let git = Git::new(&file.launch);
            let origin = git.run(&["config", "remote.origin.url"])?;
            ensure!(
                Repo::of_origin(&origin)
                    .is_some_and(|repo| repo.slug().eq_ignore_ascii_case(&file.repository)),
                "Launch origin changed"
            );
            let observed = inventory(&git)?;
            // Network reads never hold the short observation lock. A Pass can
            // keep a new blocking observation while this refresh is in flight.
            let _lock = locked(path.parent().context("no reservation directory")?)?;
            let mut current = read(&path)?;
            current.observations.retain(|row| {
                !file.observations.contains(row)
                    || observed
                        .iter()
                        .any(|(reference, sha)| reference == &row.reference && sha == &row.sha)
            });
            save(&path, &current)
        })();
        if let Err(error) = result {
            eprintln!(
                "thirdshift: could not refresh reservations in {}: {error:#}",
                path.display()
            );
        }
    }
}

fn identity(repo: &str, observation: &Observation) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!(
            "{repo}\0{}\0{}",
            observation.reference, observation.sha
        ))
    )
}

fn values(config: &UserConfig) -> Vec<(Value, PathBuf)> {
    let now = Utc::now();
    let host = crate::host::name();
    let mut rows = Vec::new();
    for path in files(config) {
        let Ok(file) = read(&path) else {
            continue;
        };
        for observation in &file.observations {
            let Some(started) = observation.owner.started() else {
                continue;
            };
            if now - started <= chrono::Duration::hours(24)
                || observation.owner.testable(host.as_deref())
            {
                continue;
            }
            let id = identity(&file.repository, observation);
            let selector = format!("reservation:{id}");
            let record = path.with_file_name(format!("reservation-{id}.json"));
            let dismissal = crate::logs::dismiss::covering(&record, &selector)
                .filter(|row| row.covers("Reservation"))
                .map(|row| row.view(None));
            let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
            let release = format!(
                "git -C {} push --no-verify --force-with-lease={}:{} origin :{}",
                quote(&file.launch.to_string_lossy()),
                observation.reference,
                observation.sha,
                observation.reference
            );
            let owner = &observation.owner;
            rows.push((json!({
                "source": "reservation", "kind": "Reservation", "name": "Reservation",
                "repository": file.repository, "selector": selector,
                "state": "held", "started_at": started.to_rfc3339(),
                "ended_at": null, "outcome": null,
                "group": if dismissal.is_some() { "resolved" } else { "needs you" },
                "needs_you_kind": if dismissal.is_some() { Value::Null } else { json!("reservation") },
                "landing": false, "stopping": false, "count": 1,
                "reservation": observation, "release_command": release,
                "blocking": observation.blocking(&file.repository),
                "owner_description": format!("Host: {}; Launch: {}; Process: {}; Started: {}", owner.host.as_deref().unwrap_or("unknown"), owner.launch.as_deref().unwrap_or("unknown"), owner.process.map_or_else(|| "unknown".into(), |pid| pid.to_string()), owner.started.as_deref().unwrap_or("unknown")),
                "dismissal": dismissal,
            }), record));
        }
    }
    rows
}

pub(crate) fn rows(config: &UserConfig) -> Vec<Value> {
    values(config).into_iter().map(|(value, _)| value).collect()
}

pub(crate) fn dismissal_target(config: &UserConfig, input: &str) -> Option<(Value, PathBuf)> {
    values(config)
        .into_iter()
        .find(|(value, _)| value["selector"] == input)
}
