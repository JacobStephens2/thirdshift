//! Extra validation a Review run asks its session to run for this repository.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::issue::Repo;

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    commands_by_repo: BTreeMap<String, Vec<String>>,
}

impl Settings {
    pub fn parse(value: &toml::Value) -> Result<Self> {
        if !value.is_table() {
            bail!("review must be the section [review]");
        }
        let settings: Self = value.clone().try_into()?;
        let mut commands_by_repo = BTreeMap::new();
        for (repo, commands) in settings.commands_by_repo {
            let parts: Vec<_> = repo.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || !part
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                })
            {
                bail!("review.commands_by_repo key {repo:?} must be owner/repo");
            }
            if commands.iter().any(|command| command.trim().is_empty()) {
                bail!("review.commands_by_repo[{repo:?}] must contain nonblank shell commands");
            }
            if commands_by_repo
                .insert(repo.to_ascii_lowercase(), commands)
                .is_some()
            {
                bail!(
                    "duplicate repository {repo:?} in review.commands_by_repo (names are case-insensitive)"
                );
            }
        }
        Ok(Self { commands_by_repo })
    }

    pub fn commands_for(&self, repo: &Repo) -> &[String] {
        self.commands_by_repo
            .get(&repo.slug().to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
}
