//! One-setting reads and edits for the CLI, Console API and User config page.
//! The Setup template inventories settings; the Run parser validates every edit.
use std::fs::{self, OpenOptions};
use std::io::Write;

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table};

use super::UserConfig;

#[derive(Serialize)]
pub struct Setting {
    pub key: String,
    pub value: Value,
    pub default: Value,
    pub editable: bool,
    pub reason: Option<&'static str>,
}

impl Setting {
    pub fn input(&self) -> String {
        if self.value.is_null() || self.key == "otlp.endpoint" {
            String::new()
        } else {
            serde_json::from_value::<toml::Value>(self.value.clone())
                .expect("a config value came from TOML")
                .to_string()
        }
    }
}

#[derive(Serialize)]
pub struct Settings {
    pub version: u8,
    pub settings: Vec<Setting>,
}

pub struct Change {
    pub old: Value,
    pub setting: Setting,
    pub changed: bool,
}

fn restriction(key: &str) -> Option<&'static str> {
    match key {
        "console.allowed_emails" => Some(
            "the Console sign-in list controls access; edit console.allowed_emails in the User config on the machine, or use setup",
        ),
        "console.address_scopes" => {
            Some("address scopes control access; use thirdshift console scopes on the machine")
        }
        _ => None,
    }
}

/// Walk the documented defaults, treating empty maps as settings themselves.
fn inventory(table: &toml::Table, prefix: &str, keys: &mut Vec<(String, Value)>) -> Result<()> {
    for (name, value) in table {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(table) = value.as_table().filter(|table| !table.is_empty()) {
            inventory(table, &key, keys)?;
        } else {
            keys.push((key, serde_json::to_value(value)?));
        }
    }
    Ok(())
}

fn lookup<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let mut parts = key.split('.');
    let mut value = table.get(parts.next()?)?;
    for part in parts {
        value = value.as_table()?.get(part)?;
    }
    Some(value)
}

fn visible(key: &str, value: Value) -> Value {
    if restriction(key).is_some() {
        return Value::Null;
    }
    // An export URL may contain a credential anywhere in its spelling.
    // Never put it in CLI/API output, an editor field or the Console log.
    if key == "otlp.endpoint" && value.as_str().is_some_and(|s| !s.is_empty()) {
        return Value::String("[redacted]".into());
    }
    value
}

fn view(table: &toml::Table) -> Result<Settings> {
    let defaults: toml::Table = super::DEFAULTS.parse()?;
    let mut keys = Vec::new();
    inventory(&defaults, "", &mut keys)?;
    keys.push(("email.to".into(), Value::Null));
    keys.sort_by(|a, b| a.0.cmp(&b.0));
    let settings = keys
        .into_iter()
        .map(|(key, default)| {
            let value = lookup(table, &key)
                .map(serde_json::to_value)
                .transpose()?
                .unwrap_or_else(|| default.clone());
            let reason = restriction(&key);
            Ok(Setting {
                value: visible(&key, value),
                key,
                default,
                editable: reason.is_none(),
                reason,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Settings {
        version: 1,
        settings,
    })
}

fn load() -> Result<(std::path::PathBuf, String)> {
    let (home, path) = super::home_and_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("can't read {}", path.display())),
    };
    UserConfig::parse(&text, &path, &home)?;
    Ok((path, text))
}

pub fn read() -> Result<Settings> {
    let (_, text) = load()?;
    view(&text.parse()?)
}

pub fn one(settings: Settings, key: &str) -> Result<Setting> {
    let setting = settings
        .settings
        .into_iter()
        .find(|setting| setting.key == key)
        .with_context(|| format!("unknown User config setting {key}"))?;
    if let Some(reason) = setting.reason {
        anyhow::bail!("can't use config for {key}: {reason}");
    }
    Ok(setting)
}

fn value(input: &str, default: &Value) -> toml::Value {
    // Bare text is convenient for strings; arrays/maps use TOML notation.
    // Invalid typed values still reach Setup's validator and its own message.
    if default.is_string() && !input.starts_with(['\'', '"', '[']) {
        return toml::Value::String(input.into());
    }
    format!("value = {input}")
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut table| table.remove("value"))
        .unwrap_or_else(|| toml::Value::String(input.into()))
}

/// Insert only the requested key, retaining existing tables and value decor.
fn put(document: &mut DocumentMut, key: &str, value: toml_edit::Value) -> Result<()> {
    let (section, name) = key.rsplit_once('.').context("a setting has a section")?;
    let mut item = document.as_item_mut();
    for part in section.split('.') {
        let table = item
            .as_table_like_mut()
            .context("a config section is a table")?;
        if !table.contains_key(part) {
            table.insert(part, Item::Table(Table::new()));
        }
        item = table.get_mut(part).expect("inserted config section");
    }
    let table = item
        .as_table_like_mut()
        .context("a config section is a table")?;
    if let Some(existing) = table.get_mut(name).and_then(Item::as_table_like_mut)
        && let Some(replacement) = value.as_inline_table()
    {
        edit_map(existing, replacement);
    } else if table.get(name).is_some_and(Item::is_value) {
        super::set(document, section, name, value);
    } else {
        table.insert(name, Item::Value(value));
    }
    Ok(())
}

fn edit_map(existing: &mut dyn toml_edit::TableLike, replacement: &dyn toml_edit::TableLike) {
    let removed: Vec<_> = existing
        .iter()
        .filter(|(key, _)| !replacement.contains_key(key))
        .map(|(key, _)| key.to_owned())
        .collect();
    for key in removed {
        existing.remove(&key);
    }
    for (key, next) in replacement.iter() {
        if let Some(old) = existing.get_mut(key) {
            if let (Some(old), Some(next)) = (old.as_table_like_mut(), next.as_table_like()) {
                edit_map(old, next);
                continue;
            }
            if let (Some(old), Some(next)) = (old.as_value_mut(), next.as_value()) {
                super::set_value(old, next.clone());
                continue;
            }
        }
        existing.insert(key, next.clone());
    }
}

/// Serialize with existing Console settings writes, validate before replacing,
/// and return safe old/new values for the caller's Console log.
pub fn write(key: &str, input: &str) -> Result<Change> {
    if let Some(reason) = restriction(key) {
        anyhow::bail!("can't use config for {key}: {reason}");
    }
    let (home, path) = super::home_and_path()?;
    let root = path.parent().expect("User config directory");
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("config.lock"))?;
    lock.lock()?;
    let (_, text) = load()?;
    let before = one(view(&text.parse()?)?, key)?;
    ensure!(
        input != "\"[redacted]\"" || key != "otlp.endpoint",
        "replace the redacted endpoint with a URL, or blank to disable export"
    );
    let next = value(input, &before.default);
    let mut document: DocumentMut = text.parse()?;
    put(&mut document, key, next.to_string().parse()?)?;
    let rendered = document.to_string();
    UserConfig::parse(&rendered, &path, &home)?;
    let changed = text.parse::<toml::Table>()? != rendered.parse::<toml::Table>()?;
    if changed {
        if path.exists() {
            super::replace(&path, &rendered)?;
        } else {
            let mut file = tempfile::NamedTempFile::new_in(root)?;
            file.write_all(rendered.as_bytes())?;
            file.persist(&path)?;
        }
    }
    Ok(Change {
        old: before.value,
        setting: one(view(&rendered.parse()?)?, key)?,
        changed,
    })
}
