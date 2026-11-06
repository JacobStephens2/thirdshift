//! Console settings and narrowly scoped, comment-preserving look writes.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::console::Scope;

pub mod fleet;
pub mod themes;

pub const THEME_FAMILIES: &[&str] = &["focus", "plain", "press", "signal", "garden", "factory"];
pub const SECTIONS: &[&str] = &[
    "needs-you",
    "ready-for-review",
    "landing",
    "working",
    "done",
    "resolved",
];

pub fn theme_names() -> Vec<String> {
    THEME_FAMILIES
        .iter()
        .flat_map(|family| ["auto", "light", "dark"].map(|mode| format!("{family}-{mode}")))
        .collect()
}

pub fn validate_theme(theme: &str) -> Result<()> {
    themes::Theme::export(theme)
        .map_err(|error| anyhow::anyhow!("unknown or invalid Console Theme {theme}: {error}"))?;
    Ok(())
}

pub fn validate_sections(sections: &[String]) -> Result<()> {
    ensure!(
        sections
            .iter()
            .all(|section| SECTIONS.contains(&section.as_str())),
        "unknown Console section; choose {}",
        SECTIONS.join(", ")
    );
    Ok(())
}

/// The Console settings Setup asks about; other Console settings stay put.
#[derive(Clone)]
pub struct ConsoleAccess {
    pub address: String,
    pub allowed_emails: Vec<String>,
    pub other_accounts: OtherAccounts,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OtherAccounts {
    Mode(AccountMode),
    Named(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountMode {
    None,
    Readable,
}

impl Default for OtherAccounts {
    fn default() -> Self {
        Self::Mode(AccountMode::None)
    }
}

impl OtherAccounts {
    pub fn parse(value: &str) -> Result<Self> {
        let setting = match value {
            "none" => Self::default(),
            "readable" => Self::Mode(AccountMode::Readable),
            _ => Self::Named(
                value
                    .split(',')
                    .map(|name| name.trim().to_owned())
                    .collect(),
            ),
        };
        setting.validate()?;
        Ok(setting)
    }

    fn validate(&self) -> Result<()> {
        if let Self::Named(names) = self {
            ensure!(
                names.iter().all(|name| !name.is_empty()
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.$".contains(&b))),
                "console.other_accounts must be none, readable, or local account names (comma-separated on the CLI)"
            );
        }
        Ok(())
    }

    pub fn value(&self) -> toml_edit::Value {
        match self {
            Self::Mode(AccountMode::None) => "none".into(),
            Self::Mode(AccountMode::Readable) => "readable".into(),
            Self::Named(names) => names.iter().collect::<toml_edit::Array>().into(),
        }
    }
}

impl std::fmt::Display for OtherAccounts {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mode(AccountMode::None) => out.write_str("none"),
            Self::Mode(AccountMode::Readable) => out.write_str("readable"),
            Self::Named(names) if names.is_empty() => out.write_str("none"),
            Self::Named(names) => out.write_str(&names.join(",")),
        }
    }
}

pub fn other_accounts(value: Option<&str>, json: bool) -> Result<String> {
    let settings = match value {
        Some(value) => edit(None, &[], &[], Some(OtherAccounts::parse(value)?), None)?,
        None => super::UserConfig::load()?.console,
    };
    Ok(if json {
        serde_json::to_string(&settings.other_accounts)?
    } else {
        settings.other_accounts.to_string()
    })
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConsoleSettings {
    pub address: String,
    pub port: u16,
    pub allowed_emails: Vec<String>,
    #[serde(skip_serializing)]
    pub address_scopes: BTreeMap<String, Vec<Scope>>,
    pub fleet: Vec<fleet::Entry>,
    #[serde(skip_serializing)]
    pub(crate) other_consoles: Vec<String>,
    pub other_accounts: OtherAccounts,
    pub theme: String,
    pub hidden_sections: Vec<String>,
}

impl Default for ConsoleSettings {
    fn default() -> Self {
        Self {
            address: String::new(),
            port: 8711,
            allowed_emails: Vec::new(),
            address_scopes: BTreeMap::new(),
            fleet: Vec::new(),
            other_consoles: Vec::new(),
            other_accounts: OtherAccounts::default(),
            theme: "focus-auto".into(),
            hidden_sections: Vec::new(),
        }
    }
}

impl ConsoleSettings {
    pub(crate) fn migrate_fleet(&mut self) -> Result<()> {
        for address in std::mem::take(&mut self.other_consoles) {
            if self.fleet.iter().any(|entry| entry.address == address) {
                continue;
            }
            let url = fleet::link_address(&address)?;
            let host = url.host_str().expect("validated host");
            let mut name = host.to_owned();
            let mut suffix = 2;
            while self.fleet.iter().any(|entry| entry.name == name) {
                name = format!("{host}-{suffix}");
                suffix += 1;
            }
            self.fleet.push(fleet::Entry::link(name, address));
        }
        Ok(())
    }

    pub(crate) fn scopes_for(&self, email: &str) -> Vec<Scope> {
        self.address_scopes
            .get(email)
            .cloned()
            .unwrap_or_else(|| Scope::ALL.to_vec())
    }

    /// The HTTPS site's DNS name, accepted with or without `https://`.
    pub fn hostname(&self) -> Result<&str> {
        let host = self
            .address
            .strip_prefix("https://")
            .unwrap_or(&self.address);
        let host = host.strip_suffix('/').unwrap_or(host);
        ensure!(
            !host.is_empty()
                && host.len() <= 253
                && host.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                }),
            "console.address must be an HTTPS DNS name, such as https://console.example.com, without a port or path"
        );
        Ok(host)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        self.other_accounts.validate()?;
        if !self.address.is_empty() {
            self.hostname()?;
        }
        let addresses = self
            .allowed_emails
            .iter()
            .map(|email| ("allowed_emails", email))
            .chain(
                self.address_scopes
                    .keys()
                    .map(|email| ("address_scopes", email)),
            );
        for (key, email) in addresses {
            ensure!(
                email.split_once('@').is_some_and(|(local, domain)| {
                    !local.is_empty()
                        && !domain.contains('@')
                        && domain.contains('.')
                        && !domain.starts_with('.')
                        && !domain.ends_with('.')
                }) && !email.chars().any(char::is_whitespace),
                "console.{key}: {email:?} is not an email address"
            );
        }
        validate_theme(&self.theme)?;
        validate_sections(&self.hidden_sections)?;
        fleet::validate(&self.fleet)?;
        Ok(())
    }
}

/// The caller can change only the two look keys. Lock the read/edit/replace
/// across CLI processes and HTTP requests so independent look writes compose.
pub fn write_look(
    theme: Option<&str>,
    hide: &[String],
    show: &[String],
) -> Result<ConsoleSettings> {
    edit(theme, hide, show, None, None)
}

/// CLI-only access change, serialized with all other Console config writes.
pub(crate) fn write_address_scopes(email: &str, scopes: Vec<Scope>) -> Result<ConsoleSettings> {
    edit(None, &[], &[], None, Some((email, scopes)))
}

fn edit(
    theme: Option<&str>,
    hide: &[String],
    show: &[String],
    other_accounts: Option<OtherAccounts>,
    address_scopes: Option<(&str, Vec<Scope>)>,
) -> Result<ConsoleSettings> {
    use toml_edit::{Item, Table, Value};

    if let Some(theme) = theme {
        validate_theme(theme)?;
    }
    validate_sections(hide)?;
    validate_sections(show)?;
    update(|settings, document| {
        if let Some(theme) = theme {
            settings.theme = theme.into();
        }
        settings
            .hidden_sections
            .retain(|section| !show.contains(section));
        for section in hide {
            if !settings.hidden_sections.contains(section) {
                settings.hidden_sections.push(section.clone());
            }
        }
        if !document.contains_key("console") {
            document["console"] = Item::Table(Table::new());
        }
        let mut set = |key: &str, value: Value| {
            let table = document["console"]
                .as_table_like_mut()
                .expect("validated console table");
            if table.contains_key(key) {
                super::set(document, "console", key, value);
            } else {
                table.insert(key, Item::Value(value));
            }
        };
        if theme.is_some() {
            set("theme", settings.theme.clone().into());
        }
        if !hide.is_empty() || !show.is_empty() {
            set(
                "hidden_sections",
                settings
                    .hidden_sections
                    .iter()
                    .collect::<toml_edit::Array>()
                    .into(),
            );
        }
        if let Some(other_accounts) = other_accounts {
            set("other_accounts", other_accounts.value());
            settings.other_accounts = other_accounts;
        }
        if let Some((email, scopes)) = address_scopes {
            ensure!(
                settings
                    .allowed_emails
                    .iter()
                    .any(|allowed| allowed == email),
                "address is not allowed in console.allowed_emails"
            );
            let table = document["console"]
                .as_table_like_mut()
                .expect("validated console table");
            if !table.contains_key("address_scopes") {
                table.insert(
                    "address_scopes",
                    Item::Value(toml_edit::InlineTable::new().into()),
                );
            }
            table
                .get_mut("address_scopes")
                .and_then(Item::as_table_like_mut)
                .expect("validated address scopes table")
                .insert(
                    email,
                    Item::Value(
                        scopes
                            .iter()
                            .map(|scope| scope.name())
                            .collect::<toml_edit::Array>()
                            .into(),
                    ),
                );
            settings.address_scopes.insert(email.to_owned(), scopes);
        }
        Ok(())
    })
}

/// Serialize comment-preserving Console writes, including Fleet migration.
fn update(
    change: impl FnOnce(&mut ConsoleSettings, &mut toml_edit::DocumentMut) -> Result<()>,
) -> Result<ConsoleSettings> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    let (home, path) = super::home_and_path()?;
    let root = path.parent().expect("User config directory");
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    lock.lock()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut settings = super::UserConfig::parse(&text, &path, &home)?.console;
    let mut document = text.parse()?;
    fleet::migrate_document(&mut document, &settings.fleet);
    change(&mut settings, &mut document)?;
    let rendered = document.to_string();
    if path.exists() {
        super::replace(&path, &rendered)?;
    } else {
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(rendered.as_bytes())?;
        file.persist(&path)?;
    }
    Ok(settings)
}
