//! The Resend API key, and where it came from: `RESEND_API_KEY` when it is
//! set and not empty, else `resend.key` in the Credentials,
//! `~/.thirdshift/credentials.toml`. The email sender and Setup both find it
//! here, so they can't disagree about whether there is one, and Setup saves
//! the key it is given here too. Only they call it, so a broken Credentials
//! file can't block a Run that sends no email, nor `update`, `version` or
//! `help`; a first Run's Setup offer, once accepted, is Setup, and reads it.

use std::fmt;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};
use toml_edit::{DocumentMut, Item};

use crate::{config, progress};

/// A Resend API key, and where it came from.
pub struct ResendKey {
    pub secret: String,
    pub source: Source,
}

/// Where a Resend API key came from.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// The `RESEND_API_KEY` environment variable.
    Environment,
    /// The Credentials at this path.
    Credentials(PathBuf),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Source::Environment => write!(f, "RESEND_API_KEY"),
            Source::Credentials(path) => write!(f, "the Credentials {}", path.display()),
        }
    }
}

impl ResendKey {
    /// The Resend API key, or `None` if there is none. The Credentials are
    /// read only when `RESEND_API_KEY` is unset or empty; a missing file
    /// holds no key, and one that others can read is used with a warning.
    /// Fails, naming the file and the key, on Credentials that aren't TOML
    /// or hold anything but a string `resend.key`, so a typo can't quietly
    /// leave a Run without its notification.
    pub fn find() -> Result<Option<Self>> {
        match from_environment() {
            Some(key) => Ok(Some(key)),
            None => Ok(Credentials::read()?.key()),
        }
    }

    /// The Resend API key, or an error listing every way to give one.
    pub fn require() -> Result<Self> {
        match ResendKey::find()? {
            Some(key) => Ok(key),
            None => bail!("{}", missing(&path()?)),
        }
    }
}

/// The key in `RESEND_API_KEY`, if it is set and not empty.
fn from_environment() -> Option<ResendKey> {
    environment_key(std::env::var("RESEND_API_KEY").ok())
}

/// The key in `RESEND_API_KEY` when it holds `value`: none if it is unset or
/// empty, so the Credentials are read instead.
fn environment_key(value: Option<String>) -> Option<ResendKey> {
    value.filter(|key| !key.is_empty()).map(|secret| ResendKey {
        secret,
        source: Source::Environment,
    })
}

/// The Credentials, as read: the file's text, if there is one, and the key
/// it holds, if any. Setup reads them before asking anything, so it can
/// refuse a broken file, and edits them to save the key it is given.
pub struct Credentials {
    path: PathBuf,
    text: Option<String>,
    key: Option<String>,
}

impl Credentials {
    /// Read the Credentials, as strictly as [`ResendKey::find`] does, with
    /// the same warning if others can read them. A missing file is no key.
    pub fn read() -> Result<Self> {
        let path = path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Credentials {
                    path,
                    text: None,
                    key: None,
                });
            }
            Err(error) => {
                return Err(error).with_context(|| format!("can't read {}", path.display()));
            }
        };
        let source = Source::Credentials(path.clone());
        let key = parse(&text, &source)?;
        warn_if_others_can_read(&path, &source);
        Ok(Credentials {
            path,
            text: Some(text),
            key,
        })
    }

    /// The Resend API key the lookup finds with these Credentials:
    /// `RESEND_API_KEY` if it is set and not empty, else theirs, if any.
    pub fn lookup(&self) -> Option<ResendKey> {
        from_environment().or_else(|| self.key())
    }

    /// The key these Credentials hold, if any.
    fn key(&self) -> Option<ResendKey> {
        self.key.clone().map(|secret| ResendKey {
            secret,
            source: Source::Credentials(self.path.clone()),
        })
    }

    /// Save `secret` as `resend.key`. With no file yet, it is created with
    /// mode 0600, and `~/.thirdshift` with it if missing, holding just the
    /// key. Otherwise the file is edited in place: its comments and anything
    /// else in it stay, and only `resend.key` changes.
    pub fn save(&self, secret: &str) -> Result<()> {
        let written = match &self.text {
            None => create(&self.path, &with_key("", secret)?),
            Some(text) => config::replace(&self.path, &with_key(text, secret)?),
        };
        written.with_context(|| format!("can't write {}", self.path.display()))
    }
}

impl fmt::Display for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "the Credentials {}", self.path.display())
    }
}

/// Whether `text` looks like a Resend API key, which starts with `re_`.
pub fn is_key(text: &str) -> bool {
    text.starts_with("re_")
}

/// `text`, Credentials that parse, with `resend.key` set to `secret`,
/// keeping the spacing and comment around the old key, if there was one.
/// With no `[resend]` section, one goes after the last line of `text`.
fn with_key(text: &str, secret: &str) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the Credentials")?;
    let Some(resend) = document.get_mut("resend") else {
        let mut section = DocumentMut::new();
        section["resend"]["key"] = toml_edit::value(secret);
        let mut text = text.to_string();
        if !text.trim().is_empty() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
        }
        text.push_str(&section.to_string());
        return Ok(text);
    };
    let resend = resend
        .as_table_like_mut()
        .context("resend must be a section in the Credentials")?;
    match resend.get_mut("key").and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = secret.into();
            *old.decor_mut() = decor;
        }
        None => {
            resend.insert("key", toml_edit::value(secret));
        }
    }
    Ok(document.to_string())
}

/// Create the file at `path` holding `text`, readable only by the user,
/// and the folder it goes in if that is missing.
fn create(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(text.as_bytes())
}

/// The Credentials' path, next to the User config.
fn path() -> Result<PathBuf> {
    Ok(config::home()?.join(".thirdshift/credentials.toml"))
}

/// `resend.key` in `text`, the contents of `file`, if it is there and not
/// empty. Any other section or key is an error, as in the User config.
fn parse(text: &str, file: &Source) -> Result<Option<String>> {
    let table: Table = text
        .parse()
        .map_err(|error| anyhow!("{error}"))
        .with_context(|| format!("can't parse {file}"))?;
    let mut found = None;
    for (section, value) in &table {
        let settings = match (section.as_str(), value) {
            ("resend", Value::Table(settings)) => settings,
            ("resend", _) => bail!("resend must be the section [resend] in {file}"),
            (_, Value::Table(_)) => bail!("unknown section [{section}] in {file}"),
            _ => bail!("unknown key {section} in {file}"),
        };
        for (key, value) in settings {
            match (key.as_str(), value) {
                ("key", Value::String(key)) => found = Some(key.clone()),
                ("key", _) => bail!("resend.key must be a quoted Resend API key in {file}"),
                _ => bail!("unknown key resend.{key} in {file}"),
            }
        }
    }
    Ok(found.filter(|key| !key.is_empty()))
}

/// Warn when `file`, at `path`, can be read by anyone but the user.
fn warn_if_others_can_read(path: &Path, file: &Source) {
    let mode = std::fs::metadata(path).map(|metadata| metadata.permissions().mode());
    if mode.is_ok_and(|mode| mode & 0o077 != 0) {
        progress::step(format_args!(
            "warning: others can read {file}; chmod 600 {}",
            path.display()
        ));
    }
}

/// What to say when there is no key, with the Credentials at `path`.
fn missing(path: &Path) -> String {
    format!(
        "no Resend API key. Either:\n  \
         - run `thirdshift setup`, or\n  \
         - add it to {} (mode 600):\n        \
         [resend]\n        \
         key = \"re_...\"\n  \
         - or set RESEND_API_KEY in the environment the Run starts from\n    \
         (a crontab line, CI secret, or a shell profile the Run's shell reads)",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_api_key_unset_or_empty_holds_no_key() {
        for value in [None, Some(String::new())] {
            assert!(environment_key(value.clone()).is_none(), "{value:?}");
        }
        let key = environment_key(Some("re_test_123".into())).unwrap();
        assert_eq!(key.secret, "re_test_123");
        assert_eq!(key.source, Source::Environment);
    }

    #[test]
    fn the_credentials_key_is_read_and_an_empty_one_is_none() {
        let file = Source::Credentials(PathBuf::from("/home/me/.thirdshift/credentials.toml"));

        assert_eq!(
            parse("[resend]\nkey = \"re_file_456\"\n", &file).unwrap(),
            Some("re_file_456".to_string())
        );
        for text in ["", "[resend]\n", "[resend]\nkey = \"\"\n"] {
            assert_eq!(parse(text, &file).unwrap(), None, "{text:?}");
        }
    }

    #[test]
    fn broken_credentials_are_an_error_naming_the_file_and_what_is_wrong() {
        let path = "/home/me/.thirdshift/credentials.toml";
        let file = Source::Credentials(PathBuf::from(path));
        for (text, named) in [
            ("[resend\n", "can't parse"),
            ("[resend]\nkye = \"re_file_456\"\n", "resend.kye"),
            ("[resend]\nkey = true\n", "resend.key"),
            ("resend = 1\n", "resend must be the section [resend]"),
            ("[email]\n", "unknown section [email]"),
            ("key = \"re_file_456\"\n", "unknown key key"),
        ] {
            let error = format!("{:#}", parse(text, &file).unwrap_err());
            for part in [named, path] {
                assert!(error.contains(part), "{text:?}: no {part:?} in {error}");
            }
        }
    }
}
