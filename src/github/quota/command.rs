//! One budget and notification total per owning Command, including subprocess
//! Ticket runs. The Command log owner supplies their shared sidecar path.
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use super::{Resource, wait::WaitBudget};
use crate::{logs, process::Interruption, progress};

#[derive(Clone, Default, Serialize, Deserialize)]
struct Totals {
    budget: WaitBudget,
    waited: Duration,
    resources: BTreeSet<String>,
}

#[derive(Default)]
struct Command {
    totals: Totals,
    path: Option<PathBuf>,
}

static COMMAND: Mutex<Option<Command>> = Mutex::new(None);

pub(crate) fn begin() {
    COMMAND.lock().unwrap().get_or_insert_with(Command::default);
}

pub(in crate::github) fn active() -> bool {
    COMMAND.lock().unwrap().is_some()
}

/// Called outside the recording lock, once its owner is known. A child uses
/// the inherited outermost owner, never its own child record's path.
pub(crate) fn attach(path: PathBuf) -> Result<()> {
    let mut command = COMMAND.lock().unwrap();
    let Some(command) = command.as_mut() else {
        return Ok(());
    };
    if command.path.is_none() {
        let totals = std::mem::take(&mut command.totals);
        command.path = Some(path.with_extension("quota-wait.json"));
        // Before log allocation, only the parent can have waited. Commands
        // that never wait need no quota sidecar.
        if !totals.budget.is_empty() {
            command.update(|stored| *stored = totals)?;
        }
    }
    Ok(())
}

impl Command {
    fn read(&self) -> Result<Totals> {
        let Some(path) = &self.path else {
            return Ok(self.totals.clone());
        };
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Totals::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn update<T>(&mut self, change: impl FnOnce(&mut Totals) -> T) -> Result<T> {
        let Some(path) = &self.path else {
            return Ok(change(&mut self.totals));
        };
        let _lock = super::store::lock_exclusive(&path.with_extension("lock"))?;
        let mut totals = self.read()?;
        let value = change(&mut totals);
        let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
        tmp.write_all(&serde_json::to_vec(&totals)?)?;
        tmp.persist(path)?;
        Ok(value)
    }
}

/// The ordinary sleep and the completion sleep share accounting but preserve
/// their caller's interruption policy. A rejected reservation leaves the
/// caller's original GitHub failure intact.
pub(in crate::github) fn wait(
    reset: DateTime<Utc>,
    resource: Resource,
    interruption: Interruption,
) -> Result<bool> {
    let delay = {
        let mut command = COMMAND.lock().unwrap();
        let Some(command) = command.as_mut() else {
            return Ok(false);
        };
        command.update(|totals| totals.budget.next(reset.max(Utc::now())))?
    };
    let Ok(delay) = delay else { return Ok(false) };
    let until = (Utc::now() + chrono::Duration::from_std(delay)?)
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    logs::quota_wait(Some(&until));
    let started = Instant::now();
    let mut left = delay;
    let result = loop {
        progress::step(format!("Quota wait until {until} ({})", resource.name()));
        if let Err(error) = interruption.pause(left.min(Duration::from_secs(30))) {
            break Err(error);
        }
        left = delay.saturating_sub(started.elapsed());
        if left.is_zero() {
            break Ok(());
        }
    };
    logs::quota_wait(None);
    let elapsed = started.elapsed().min(delay);
    if let Some(command) = COMMAND.lock().unwrap().as_mut() {
        command.update(|totals| {
            totals.budget.refund(delay.saturating_sub(elapsed));
            totals.waited += elapsed;
            if !elapsed.is_zero() {
                totals.resources.insert(resource.name().into());
            }
        })?;
    }
    result.map(|()| true)
}

pub(crate) fn notification_line() -> Result<Option<String>> {
    let command = COMMAND.lock().unwrap();
    let Some(command) = command.as_ref() else {
        return Ok(None);
    };
    let totals = command.read()?;
    Ok((!totals.waited.is_zero()).then(|| {
        format!(
            "Waited {} for the GitHub quota ({})",
            progress::duration(totals.waited),
            totals
                .resources
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    #[test]
    fn quota_wait_sidecar_lock_stays_private_to_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.quota-wait.json");
        let lock_path = path.with_extension("lock");
        fs::write(&lock_path, []).unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut command = Command {
            totals: Totals::default(),
            path: Some(path),
        };
        command
            .update(|totals| {
                totals.waited = Duration::from_secs(1);
            })
            .unwrap();
        let mode = fs::metadata(&lock_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "sidecar lock mode {mode:o} lets another local user block the owner"
        );
    }
}
