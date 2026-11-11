//! One machine-local record, with a stable lock across atomic replacements.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

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
            .filter(|wait| &wait.account == account && wait.quota.reset_at > now)
            .collect())
    }

    /// A successful quota observation supersedes the previous one, including
    /// clearing a wait when the reserve is available again.
    pub fn observe(
        &self,
        account: &Account,
        resource: Resource,
        quota: Quota,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let waiting = quota.low(now);
        self.record(account, resource, quota, waiting, now)
    }

    pub fn exhausted(
        &self,
        account: &Account,
        resource: Resource,
        quota: Quota,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.record(account, resource, quota, true, now)
    }

    fn record(
        &self,
        account: &Account,
        resource: Resource,
        quota: Quota,
        waiting: bool,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.update(|record| {
            record.waits.retain(|wait| {
                wait.quota.reset_at > now
                    && !(&wait.account == account && wait.resource == resource)
            });
            if waiting && quota.reset_at > now {
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
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        lock.lock()?;
        let mut record = self.read()?;
        change(&mut record);
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(&serde_json::to_vec(&record)?)?;
        tmp.persist(&self.path)?;
        Ok(())
    }
}
