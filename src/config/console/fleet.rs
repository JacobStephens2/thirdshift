//! Fleet entries contain only shareable identity and last-contact metadata.
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table};

use crate::console::Scope;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thirdshift_version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_contact: Option<DateTime<Utc>>,
}

impl Entry {
    pub fn link(name: String, address: String) -> Self {
        Self {
            name,
            address,
            hostname: None,
            thirdshift_version: None,
            scopes: Vec::new(),
            last_contact: None,
        }
    }
}

pub(super) fn validate(entries: &[Entry]) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for entry in entries {
        validate_name(&entry.name)?;
        ensure!(
            names.insert(&entry.name),
            "duplicate console.fleet name {:?}",
            entry.name
        );
        link_address(&entry.address)?;
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty() && !name.starts_with('-') && !name.chars().any(char::is_control),
        "Fleet name must be nonempty and printable, without a leading '-'"
    );
    Ok(())
}

pub(crate) fn link_address(address: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(address)?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "console.fleet must contain http or https addresses"
    );
    Ok(url)
}

/// Pair or replace one named machine without exposing the config's TOML.
pub(crate) fn pair(entry: &Entry) -> Result<()> {
    change_entry(&entry.name, Some(entry))
}

pub(crate) fn remove(name: &str) -> Result<()> {
    change_entry(name, None)
}

fn change_entry(name: &str, entry: Option<&Entry>) -> Result<()> {
    super::update(|settings, document| {
        settings.fleet.retain(|saved| saved.name != name);
        if let Some(entry) = entry {
            settings.fleet.push(entry.clone());
        }
        write_document(document, &settings.fleet)
    })?;
    Ok(())
}

fn write_document(document: &mut DocumentMut, entries: &[Entry]) -> Result<()> {
    if !document.contains_key("console") {
        document["console"] = Item::Table(Table::new());
    }
    let table = document["console"]
        .as_table_like_mut()
        .expect("validated console table");
    table.remove("other_consoles");
    let value: DocumentMut = toml::to_string(&Fleet { fleet: entries })?.parse()?;
    table.insert("fleet", value["fleet"].clone());
    Ok(())
}

#[derive(Serialize)]
struct Fleet<'a> {
    fleet: &'a [Entry],
}

pub(crate) fn migrate_document(document: &mut DocumentMut, entries: &[Entry]) {
    if document
        .get("console")
        .and_then(Item::as_table_like)
        .is_some_and(|table| table.contains_key("other_consoles"))
    {
        write_document(document, entries).expect("Fleet entries serialize to TOML");
    }
}
