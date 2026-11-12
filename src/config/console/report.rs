//! Outbound report destinations: Consoles this machine pushes its own Run
//! metrics summary to, each under the name its `report` token was issued
//! for. Only shareable names and addresses; tokens live in Credentials.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    /// This machine's name for the receiving Console.
    pub name: String,
    pub address: String,
    /// The Fleet name the receiving Console issued the `report` token to:
    /// the machine every push names.
    pub holder: String,
}

pub(super) fn validate(destinations: &[Destination]) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for destination in destinations {
        super::fleet::validate_name(&destination.name)?;
        super::fleet::validate_name(&destination.holder)?;
        ensure!(
            names.insert(&destination.name),
            "duplicate console.report name {:?}",
            destination.name
        );
        crate::console::fleet::pairing_address(&destination.address)?;
    }
    Ok(())
}

/// Add or replace one named destination without exposing the config's TOML.
pub(crate) fn pair(destination: &Destination) -> Result<()> {
    change(&destination.name, Some(destination))
}

pub(crate) fn remove(name: &str) -> Result<()> {
    change(name, None)
}

fn change(name: &str, destination: Option<&Destination>) -> Result<()> {
    super::update(|settings, document| {
        settings.report.retain(|saved| saved.name != name);
        if let Some(destination) = destination {
            settings.report.push(destination.clone());
        }
        if !document.contains_key("console") {
            document["console"] = Item::Table(Table::new());
        }
        let table = document["console"]
            .as_table_like_mut()
            .expect("validated console table");
        let value: DocumentMut = toml::to_string(&Report {
            report: &settings.report,
        })?
        .parse()?;
        table.insert("report", value["report"].clone());
        Ok(())
    })?;
    Ok(())
}

#[derive(Serialize)]
struct Report<'a> {
    report: &'a [Destination],
}
