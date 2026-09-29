//! The User config: `~/.thirdshift/config.toml`, this machine's defaults for
//! every Run. Only a Run reads it, so a broken one can't block `update`,
//! `version` or `help`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};

use crate::run::Goal;

/// The settings a User config can hold. Each is what a Run does when its
/// command says nothing about it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UserConfig {
    /// `merge.always`: every Run is a Merge run unless told `no-merge`.
    pub merge_always: bool,
    /// `launch.pull`: every Run fast-forwards the Launch directory's
    /// checkout of the Base branch to `origin`.
    pub launch_pull: bool,
}

impl UserConfig {
    /// The User config under `$HOME`, or the defaults if there is none.
    pub fn load() -> Result<Self> {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return Ok(UserConfig::default());
        };
        let path = PathBuf::from(home).join(".thirdshift/config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => UserConfig::parse(&text, &path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UserConfig::default()),
            Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
        }
    }

    /// Parse `text`, the contents of the User config at `path`. Any key or
    /// section thirdshift doesn't know is an error, so a typo can't silently
    /// leave a setting unset.
    fn parse(text: &str, path: &Path) -> Result<Self> {
        let file = format!("the User config {}", path.display());
        let table: Table = text
            .parse()
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("can't parse {file}"))?;
        let mut config = UserConfig::default();
        for (section, value) in &table {
            let settings = match (section.as_str(), value) {
                ("merge" | "launch", Value::Table(settings)) => settings,
                ("merge" | "launch", _) => {
                    bail!("{section} must be the section [{section}] in {file}")
                }
                (_, Value::Table(_)) => bail!("unknown section [{section}] in {file}"),
                _ => bail!("unknown key {section} in {file}"),
            };
            for (key, value) in settings {
                let setting = match (section.as_str(), key.as_str()) {
                    ("merge", "always") => &mut config.merge_always,
                    ("launch", "pull") => &mut config.launch_pull,
                    _ => bail!("unknown key {section}.{key} in {file}"),
                };
                let Value::Boolean(on) = value else {
                    bail!("{section}.{key} must be true or false in {file}");
                };
                *setting = *on;
            }
        }
        Ok(config)
    }

    /// The goal of a Run whose command gave no `merge` or `no-merge`.
    pub fn default_goal(&self) -> Goal {
        if self.merge_always {
            Goal::Merged
        } else {
            Goal::ReadyForReview
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<UserConfig> {
        UserConfig::parse(text, Path::new("/home/me/.thirdshift/config.toml"))
    }

    #[test]
    fn merge_always_is_read() {
        assert!(parse("[merge]\nalways = true\n").unwrap().merge_always);
        assert!(!parse("[merge]\nalways = false\n").unwrap().merge_always);
        assert!(!parse("").unwrap().merge_always);
    }

    #[test]
    fn launch_pull_is_read() {
        assert!(parse("[launch]\npull = true\n").unwrap().launch_pull);
        assert!(!parse("[launch]\npull = false\n").unwrap().launch_pull);
        assert!(!parse("").unwrap().launch_pull);
    }

    #[test]
    fn errors_name_the_file_and_the_offending_key() {
        for (text, named) in [
            ("[merge\n", "can't parse the User config"),
            ("[merge]\nalway = true\n", "unknown key merge.alway"),
            ("[logs]\n", "unknown section [logs]"),
            ("merge = true\n", "merge must be the section [merge]"),
            ("colour = true\n", "unknown key colour "),
            (
                "[merge]\nalways = 1\n",
                "merge.always must be true or false",
            ),
            ("[launch]\npul = true\n", "unknown key launch.pul"),
            ("launch = true\n", "launch must be the section [launch]"),
            (
                "[launch]\npull = \"yes\"\n",
                "launch.pull must be true or false",
            ),
        ] {
            let error = format!("{:#}", parse(text).unwrap_err());
            assert!(error.contains(named), "{text:?}: {error}");
            assert!(
                error.contains("/home/me/.thirdshift/config.toml"),
                "{text:?}: {error}"
            );
        }
    }
}
