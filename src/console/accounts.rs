//! Durable Console accounts, shared by the server and CLI under one file lock.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const LIFETIME: i64 = 30 * 24 * 60 * 60;

#[derive(Default, Deserialize, Serialize)]
struct Account {
    password_hash: String,
    #[serde(default)]
    sessions: BTreeMap<String, i64>,
    #[serde(default)]
    failed_passwords: u32,
    #[serde(default)]
    next_attempt: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum EventKind {
    SignIn,
    FailedSignIn,
    PasswordSet,
    SignOut,
    BlockedAddress,
    ThemeChanged,
    SectionsChanged,
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::SignIn => "sign-in",
            Self::FailedSignIn => "failed-sign-in",
            Self::PasswordSet => "password-set",
            Self::SignOut => "sign-out",
            Self::BlockedAddress => "blocked-address",
            Self::ThemeChanged => "theme-changed",
            Self::SectionsChanged => "sections-changed",
        })
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct Event {
    pub time: i64,
    pub email: String,
    pub event: EventKind,
}

pub(super) enum SignIn {
    SignedIn(String),
    Refused,
    Slowdown,
}

pub(super) struct Accounts {
    root: PathBuf,
    // Keep a separate, stable inode locked across account-file replacements.
    _lock: File,
    allowed: Vec<String>,
    now: i64,
    accounts: BTreeMap<String, Account>,
}

impl Accounts {
    pub fn load() -> Result<Self> {
        let config = crate::config::UserConfig::load()?;
        Self::open(
            &crate::config::home()?.join(".thirdshift"),
            &config.console.allowed_emails,
            chrono::Utc::now().timestamp(),
        )
    }

    pub fn open(root: &Path, allowed: &[String], now: i64) -> Result<Self> {
        fs::create_dir_all(root)?;
        let lock = private_file(&root.join("console-accounts.lock"))?;
        lock.lock()?;
        let path = root.join("console-accounts.json");
        let accounts = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid Console accounts")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        let mut store = Self {
            root: root.into(),
            _lock: lock,
            allowed: allowed.to_vec(),
            now,
            accounts,
        };
        let mut changed = false;
        let mut blocked = Vec::new();
        for (email, account) in &mut store.accounts {
            let before = account.sessions.len();
            if !allowed.contains(email) {
                account.sessions.clear();
                if before != 0 {
                    blocked.push(email.clone());
                }
            } else {
                account
                    .sessions
                    .retain(|_, last_use| now.saturating_sub(*last_use) < LIFETIME);
            }
            changed |= before != account.sessions.len();
        }
        if changed {
            store.save()?;
        }
        for email in blocked {
            store.record(&email, EventKind::BlockedAddress)?;
        }
        Ok(store)
    }

    pub fn set_password(&mut self, email: &str, password: &str) -> Result<()> {
        ensure!(
            self.allowed.iter().any(|allowed| allowed == email),
            "address is not allowed in console.allowed_emails"
        );
        ensure!(!password.is_empty(), "password must not be empty");
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|error| anyhow::anyhow!("can't hash password: {error}"))?
            .to_string();
        self.accounts.insert(
            email.into(),
            Account {
                password_hash,
                ..Account::default()
            },
        );
        self.save()?;
        self.record(email, EventKind::PasswordSet)
    }

    pub fn sign_in(&mut self, email: &str, password: &str) -> Result<SignIn> {
        if !self.allowed.iter().any(|allowed| allowed == email) {
            self.record(email, EventKind::BlockedAddress)?;
            return Ok(SignIn::Refused);
        }
        if self
            .accounts
            .get(email)
            .is_some_and(|account| self.now < account.next_attempt)
        {
            self.record(email, EventKind::FailedSignIn)?;
            return Ok(SignIn::Slowdown);
        }
        let valid = self.accounts.get(email).is_some_and(|account| {
            PasswordHash::new(&account.password_hash).is_ok_and(|hash| {
                Argon2::default()
                    .verify_password(password.as_bytes(), &hash)
                    .is_ok()
            })
        });
        if !valid {
            if let Some(account) = self.accounts.get_mut(email) {
                account.failed_passwords = account.failed_passwords.saturating_add(1);
                if account.failed_passwords >= 5 {
                    account.next_attempt = self.now + 60;
                }
                self.save()?;
            }
            self.record(email, EventKind::FailedSignIn)?;
            return Ok(SignIn::Refused);
        }
        let token = random_token();
        let account = self.accounts.get_mut(email).expect("verified account");
        account.failed_passwords = 0;
        account.next_attempt = 0;
        account.sessions.insert(digest(&token), self.now);
        self.save()?;
        self.record(email, EventKind::SignIn)?;
        Ok(SignIn::SignedIn(token))
    }

    pub fn session(&mut self, token: &str, touch: bool) -> Result<Option<String>> {
        let hash = digest(token);
        let email = self.accounts.iter().find_map(|(email, account)| {
            account.sessions.contains_key(&hash).then(|| email.clone())
        });
        if let Some(email) = &email
            && touch
        {
            self.accounts
                .get_mut(email)
                .expect("session account")
                .sessions
                .insert(hash, self.now);
            self.save()?;
        }
        Ok(email)
    }

    // None ends every session for the account (CLI); Some ends this browser only.
    pub fn sign_out(&mut self, email: &str, token: Option<&str>) -> Result<()> {
        if let Some(account) = self.accounts.get_mut(email) {
            match token {
                Some(token) => {
                    account.sessions.remove(&digest(token));
                }
                None => account.sessions.clear(),
            }
            self.save()?;
        }
        self.record(email, EventKind::SignOut)
    }

    pub fn events(&self) -> Result<Vec<Event>> {
        let text = match fs::read_to_string(self.root.join("console.log")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        text.lines()
            .map(|line| serde_json::from_str(line).context("invalid Console log event"))
            .collect()
    }

    fn save(&self) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        file.write_all(&serde_json::to_vec(&self.accounts)?)?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("console-accounts.json"))?;
        Ok(())
    }

    pub(super) fn record(&self, email: &str, event: EventKind) -> Result<()> {
        let mut file = private_file(&self.root.join("console.log"))?;
        serde_json::to_writer(
            &mut file,
            &Event {
                time: self.now,
                email: email.into(),
                event,
            },
        )?;
        file.write_all(b"\n")?;
        Ok(())
    }
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

pub(super) fn random_token() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests;
