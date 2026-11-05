//! CLI-only token management and the hash-only machine store, serialized by a
//! stable lock across atomic replacements, including HTTP last-use updates.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use super::{Scope, accounts};

pub(crate) enum Action {
    Create {
        holder: String,
        scopes: Vec<Scope>,
        expires_at: Option<DateTime<Utc>>,
    },
    List {
        json: bool,
    },
    Revoke(String),
}

pub(crate) fn parse(args: &[String]) -> Result<Action> {
    let usage = "usage: thirdshift console token create <holder> [--scopes read,security,act,run,admin] [--expires <date>] | list [--json] | revoke <holder>";
    match args.first().map(String::as_str) {
        Some("create") => {
            let holder = args
                .get(1)
                .filter(|name| !name.starts_with('-'))
                .context(usage)?;
            ensure!(
                !holder.trim().is_empty() && !holder.chars().any(char::is_control),
                "holder must be a nonempty printable name"
            );
            let mut scopes = None;
            let mut expires_at = None;
            let mut options = args[2..].iter();
            while let Some(flag) = options.next() {
                let value = options.next().context(usage)?;
                match flag.as_str() {
                    "--scopes" if scopes.is_none() => {
                        let mut selected = Vec::new();
                        for name in value.split(',') {
                            let scope = Scope::ALL
                                .into_iter()
                                .find(|scope| scope.name() == name)
                                .with_context(|| {
                                format!(
                                    "unknown scope {name:?}; expected read,security,act,run,admin"
                                )
                            })?;
                            if !selected.contains(&scope) {
                                selected.push(scope);
                            }
                        }
                        scopes = Some(selected);
                    }
                    "--expires" if expires_at.is_none() => {
                        expires_at = Some(expiry(value)?);
                    }
                    _ => bail!("{usage}"),
                }
            }
            Ok(Action::Create {
                holder: holder.clone(),
                scopes: scopes.unwrap_or_else(|| vec![Scope::Read]),
                expires_at,
            })
        }
        Some("list") => match &args[1..] {
            [] => Ok(Action::List { json: false }),
            [flag] if flag == "--json" => Ok(Action::List { json: true }),
            _ => bail!("{usage}"),
        },
        Some("revoke") => {
            let [holder] = &args[1..] else {
                bail!("{usage}")
            };
            Ok(Action::Revoke(holder.clone()))
        }
        _ => bail!("{usage}"),
    }
}

fn expiry(value: &str) -> Result<DateTime<Utc>> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date
            .and_hms_opt(0, 0, 0)
            .context("invalid expiry date")?
            .and_utc());
    }
    DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc))
        .context("expiry must be YYYY-MM-DD (start of day, UTC) or an RFC 3339 timestamp")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Metadata {
    pub holder: String,
    pub scopes: Vec<Scope>,
    issued_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
    last_use: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Serialize)]
struct Stored {
    #[serde(flatten)]
    metadata: Metadata,
    sha256: String,
}

struct Tokens {
    root: PathBuf,
    _lock: File,
    records: BTreeMap<String, Stored>,
}

impl Tokens {
    fn load() -> Result<Self> {
        let root = crate::config::home()?.join(".thirdshift");
        fs::create_dir_all(&root)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join("console-tokens.lock"))?;
        lock.set_permissions(fs::Permissions::from_mode(0o600))?;
        lock.lock()?;
        let records = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join("console-tokens.json"))
        {
            Ok(file) => {
                file.set_permissions(fs::Permissions::from_mode(0o600))?;
                serde_json::from_reader(file).context("invalid Console token store")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            root,
            _lock: lock,
            records,
        })
    }

    fn save(&self) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut file, &self.records)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("console-tokens.json"))?;
        Ok(())
    }
}

pub(crate) fn run(action: Action) -> Result<()> {
    let mut tokens = Tokens::load()?;
    match action {
        Action::Create {
            holder,
            scopes,
            expires_at,
        } => {
            ensure!(
                !tokens.records.contains_key(&holder),
                "token holder already exists; revoke it first"
            );
            let secret = format!("ts_{}", accounts::random_token());
            tokens.records.insert(
                holder.clone(),
                Stored {
                    metadata: Metadata {
                        holder,
                        scopes,
                        issued_at: Utc::now(),
                        expires_at,
                        last_use: None,
                    },
                    sha256: accounts::digest(&secret),
                },
            );
            tokens.save()?;
            println!("{secret}");
        }
        Action::List { json } => {
            let metadata: Vec<_> = tokens
                .records
                .values()
                .map(|stored| &stored.metadata)
                .collect();
            if json {
                println!("{}", serde_json::to_string(&metadata)?);
            } else {
                println!("Holder\tScopes\tIssued\tExpires\tLast use");
                for record in metadata {
                    println!(
                        "{}\t{}\t{}\t{}\t{}",
                        record.holder,
                        record
                            .scopes
                            .iter()
                            .map(|scope| scope.name())
                            .collect::<Vec<_>>()
                            .join(","),
                        record.issued_at.to_rfc3339(),
                        record
                            .expires_at
                            .map(|date| date.to_rfc3339())
                            .unwrap_or_else(|| "never".into()),
                        record
                            .last_use
                            .map(|date| date.to_rfc3339())
                            .unwrap_or_else(|| "never".into())
                    );
                }
            }
        }
        Action::Revoke(holder) => {
            ensure!(
                tokens.records.remove(&holder).is_some(),
                "unknown token holder"
            );
            tokens.save()?;
            println!("Revoked {holder}.");
        }
    }
    Ok(())
}

pub(super) fn authenticate(secret: &str) -> Result<Option<Metadata>> {
    let mut tokens = Tokens::load()?;
    let digest = accounts::digest(secret);
    let now = Utc::now();
    let selected = tokens.records.values_mut().find(|record| {
        bool::from(record.sha256.as_bytes().ct_eq(digest.as_bytes()))
            && record.metadata.expires_at.is_none_or(|expiry| now < expiry)
    });
    let Some(selected) = selected else {
        return Ok(None);
    };
    selected.metadata.last_use = Some(now);
    let metadata = selected.metadata.clone();
    tokens.save()?;
    Ok(Some(metadata))
}
