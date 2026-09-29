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
    /// `logs.dir`, with a leading `~` expanded: where session logs are
    /// written instead of `~/.thirdshift/logs`.
    pub logs_dir: Option<PathBuf>,
}

impl UserConfig {
    /// The User config under `$HOME`, or the defaults if there is none.
    pub fn load() -> Result<Self> {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return Ok(UserConfig::default());
        };
        let home = PathBuf::from(home);
        let path = home.join(".thirdshift/config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => UserConfig::parse(&text, &path, &home),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UserConfig::default()),
            Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
        }
    }

    /// Parse `text`, the contents of the User config at `path`, expanding a
    /// leading `~` to `home`. Any key or section thirdshift doesn't know is an
    /// error, so a typo can't silently leave a setting unset.
    fn parse(text: &str, path: &Path, home: &Path) -> Result<Self> {
        let file = format!("the User config {}", path.display());
        let table: Table = text
            .parse()
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("can't parse {file}"))?;
        let mut config = UserConfig::default();
        for (section, value) in &table {
            let known = matches!(section.as_str(), "merge" | "logs");
            let settings = match value {
                Value::Table(settings) if known => settings,
                Value::Table(_) => bail!("unknown section [{section}] in {file}"),
                _ if known => bail!("{section} must be the section [{section}] in {file}"),
                _ => bail!("unknown key {section} in {file}"),
            };
            for (key, value) in settings {
                match (section.as_str(), key.as_str(), value) {
                    ("merge", "always", Value::Boolean(always)) => config.merge_always = *always,
                    ("merge", "always", _) => bail!("merge.always must be true or false in {file}"),
                    ("logs", "dir", Value::String(dir)) => match expand_home(dir, home) {
                        Some(dir) => config.logs_dir = Some(dir),
                        None => bail!(
                            "logs.dir must be an absolute path or start with ~/, not {dir:?}, in {file}"
                        ),
                    },
                    ("logs", "dir", _) => bail!("logs.dir must be a string in {file}"),
                    _ => bail!("unknown key {section}.{key} in {file}"),
                }
            }
        }
        Ok(config)
    }

    /// Where a Run writes its session logs: `logs.dir`, or by default
    /// `~/.thirdshift/logs`.
    pub fn logs_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = &self.logs_dir {
            return Ok(dir.clone());
        }
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(PathBuf::from(home).join(".thirdshift/logs"))
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

/// `path` as an absolute path, with a leading `~` expanded to `home`, or
/// `None` if it is relative: the directory a Run is launched from is no base
/// for a setting that holds for every Run.
fn expand_home(path: &str, home: &Path) -> Option<PathBuf> {
    let path = match path.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) => home.join(rest.strip_prefix('/')?),
        None => PathBuf::from(path),
    };
    path.is_absolute().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<UserConfig> {
        UserConfig::parse(
            text,
            Path::new("/home/me/.thirdshift/config.toml"),
            Path::new("/home/me"),
        )
    }

    #[test]
    fn merge_always_is_read() {
        assert!(parse("[merge]\nalways = true\n").unwrap().merge_always);
        assert!(!parse("[merge]\nalways = false\n").unwrap().merge_always);
        assert!(!parse("").unwrap().merge_always);
    }

    #[test]
    fn logs_dir_is_read_with_a_leading_tilde_expanded() {
        for (dir, expanded) in [
            ("~/elsewhere/logs", "/home/me/elsewhere/logs"),
            ("~", "/home/me"),
            ("/var/log/thirdshift", "/var/log/thirdshift"),
        ] {
            let config = parse(&format!("[logs]\ndir = {dir:?}\n")).unwrap();
            assert_eq!(config.logs_dir, Some(PathBuf::from(expanded)), "{dir}");
        }
        assert_eq!(parse("[logs]\n").unwrap().logs_dir, None);
        assert_eq!(parse("").unwrap().logs_dir, None);
    }

    #[test]
    fn errors_name_the_file_and_the_offending_key() {
        for (text, named) in [
            ("[merge\n", "can't parse the User config"),
            ("[merge]\nalway = true\n", "unknown key merge.alway"),
            ("[log]\n", "unknown section [log]"),
            (
                "[logs]\ndirectory = \"/tmp\"\n",
                "unknown key logs.directory",
            ),
            ("logs = \"/tmp\"\n", "logs must be the section [logs]"),
            ("[logs]\ndir = 3\n", "logs.dir must be a string"),
            (
                "[logs]\ndir = \"logs\"\n",
                "logs.dir must be an absolute path",
            ),
            (
                "[logs]\ndir = \"./logs\"\n",
                "logs.dir must be an absolute path",
            ),
            (
                "[logs]\ndir = \"~other/logs\"\n",
                "logs.dir must be an absolute path",
            ),
            ("[logs]\ndir = \"\"\n", "logs.dir must be an absolute path"),
            ("merge = true\n", "merge must be the section [merge]"),
            ("colour = true\n", "unknown key colour "),
            (
                "[merge]\nalways = 1\n",
                "merge.always must be true or false",
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
