//! One machine-local record, with a stable lock across atomic replacements.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Quota, Resource};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Account {
    host: String,
    login: String,
}

impl Account {
    pub fn new(host: &str, login: &str) -> Self {
        Self {
            host: host.to_ascii_lowercase(),
            login: login.to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct QuotaWait {
    #[serde(flatten)]
    account: Account,
    pub resource: Resource,
    #[serde(flatten)]
    pub quota: Quota,
}

#[derive(Serialize, Deserialize)]
struct Identity {
    credential: String,
    account: Account,
}

#[derive(Default, Serialize, Deserialize)]
struct Record {
    identities: Vec<Identity>,
    waits: Vec<QuotaWait>,
}

pub(crate) struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn machine() -> Result<Self> {
        Ok(Self::new(
            crate::config::home()?.join(".thirdshift/github-quota.json"),
        ))
    }

    pub fn waiting(&self, account: &Account, now: DateTime<Utc>) -> Result<Vec<QuotaWait>> {
        Ok(self
            .read()?
            .waits
            .into_iter()
            // A wait recorded with the reserve available, as after a
            // window had already reset, is not a wait.
            .filter(|wait| &wait.account == account && wait.quota.low(now))
            .collect())
    }

    /// A quota observation supersedes the previous one: it records a wait
    /// only while the quota is low, and otherwise clears any stored wait.
    pub fn observe(
        &self,
        account: &Account,
        resource: Resource,
        quota: Quota,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.update(|record| {
            record.waits.retain(|wait| {
                wait.quota.reset_at > now
                    && !(&wait.account == account && wait.resource == resource)
            });
            if quota.low(now) {
                record.waits.push(QuotaWait {
                    account: account.clone(),
                    resource,
                    quota,
                });
            }
        })
    }

    pub fn account(&self, host: &str, credential: &str) -> Result<Option<Account>> {
        Ok(self
            .read()?
            .identities
            .into_iter()
            .find(|identity| identity.account.host == host && identity.credential == credential)
            .map(|identity| identity.account))
    }

    pub fn remember(&self, account: Account, credential: String) -> Result<()> {
        self.update(|record| {
            record.identities.retain(|identity| {
                identity.account.host != account.host || identity.credential != credential
            });
            record.identities.push(Identity {
                account,
                credential,
            });
        })
    }

    fn read(&self) -> Result<Record> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Record::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn update(&self, change: impl FnOnce(&mut Record)) -> Result<()> {
        let dir = self.path.parent().expect("quota record has a parent");
        fs::create_dir_all(dir)?;
        let _lock = lock_exclusive(&self.path.with_extension("lock"))?;
        let mut record = self.read()?;
        let previous = serde_json::to_vec(&record)?;
        change(&mut record);
        let bytes = serde_json::to_vec(&record)?;
        // The Console watches this record: unchanged quota observations
        // must not wake open Inbox pages.
        if bytes == previous {
            return Ok(());
        }
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&bytes)?;
        tmp.persist(&self.path)?;
        Ok(())
    }
}

/// Another local user can take an exclusive flock on a lock they can open,
/// even with a read-only descriptor, and stall every Quota update. Creation
/// mode is masked by umask, so an existing lock is tightened explicitly.
pub(super) fn lock_exclusive(path: &Path) -> Result<File> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(path)?;
    lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    lock.lock()?;
    Ok(lock)
}
