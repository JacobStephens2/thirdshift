//! Extra validation a Review run asks its session to run for this repository.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::issue::Repo;

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub merge: bool,
    commands_by_repo: BTreeMap<String, Vec<String>>,
}

impl Settings {
    pub fn parse(value: &toml::Value) -> Result<Self> {
        if !value.is_table() {
            bail!("review must be the section [review]");
        }
        let mut settings: Self = value.clone().try_into()?;
        settings.commands_by_repo =
            super::repository_keys(settings.commands_by_repo, "review.commands_by_repo")?;
        for (repo, commands) in &settings.commands_by_repo {
            if commands.iter().any(|command| command.trim().is_empty()) {
                bail!("review.commands_by_repo[{repo:?}] must contain nonblank shell commands");
            }
        }
        Ok(settings)
    }

    pub fn commands_for(&self, repo: &Repo) -> &[String] {
        self.commands_by_repo
            .get(&repo.slug().to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
}
