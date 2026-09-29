//! The User config: `~/.thirdshift/config.toml`, this machine's defaults for
//! every Run. Only a Run and `email-test` read it, so a broken one can't block
//! `update`, `version` or `help`.

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
    /// The `[email]` section.
    pub email: EmailSettings,
}

/// The `[email]` section: where email goes and who it comes from. The Resend
/// API key is never here, only in `RESEND_API_KEY`, so the file holds no
/// secret.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmailSettings {
    /// `email.to`: the address email goes to when the command names none.
    pub to: Option<String>,
    /// `email.from`: the sender, in place of Resend's shared test sender.
    pub from: Option<String>,
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
                ("merge" | "email", Value::Table(settings)) => settings,
                ("merge" | "email", _) => {
                    bail!("{section} must be the section [{section}] in {file}")
                }
                (_, Value::Table(_)) => bail!("unknown section [{section}] in {file}"),
                _ => bail!("unknown key {section} in {file}"),
            };
            for (key, value) in settings {
                match (section.as_str(), key.as_str(), value) {
                    ("merge", "always", Value::Boolean(always)) => config.merge_always = *always,
                    ("merge", "always", _) => {
                        bail!("merge.always must be true or false in {file}")
                    }
                    ("email", "to", Value::String(to)) => config.email.to = Some(to.clone()),
                    ("email", "from", Value::String(from)) => {
                        config.email.from = Some(from.clone())
                    }
                    ("email", "to" | "from", _) => {
                        bail!("{section}.{key} must be a quoted email address in {file}")
                    }
                    _ => bail!("unknown key {section}.{key} in {file}"),
                }
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
    fn email_settings_are_read() {
        let config = parse("[email]\nto = \"me@example.com\"\nfrom = \"ts@acme.dev\"\n").unwrap();
        assert_eq!(config.email.to.as_deref(), Some("me@example.com"));
        assert_eq!(config.email.from.as_deref(), Some("ts@acme.dev"));
        assert_eq!(parse("[email]\n").unwrap().email, EmailSettings::default());
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
            ("[email]\nadress = \"a@b.c\"\n", "unknown key email.adress"),
            ("email = \"a@b.c\"\n", "email must be the section [email]"),
            (
                "[email]\nto = 1\n",
                "email.to must be a quoted email address",
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
