//! The Resend API key, and where it came from: `RESEND_API_KEY` when it is
//! set and not empty, else `resend.key` in the Credentials,
//! `~/.thirdshift/credentials.toml`. The email sender and Setup both find it
//! here, so they can't disagree about whether there is one. Only they call
//! it, so a broken Credentials file can't block a Run that sends no email,
//! nor `update`, `version` or `help`.

use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};

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
        let key = std::env::var("RESEND_API_KEY")
            .ok()
            .filter(|key| !key.is_empty());
        if let Some(key) = key {
            return Ok(Some(ResendKey {
                secret: key,
                source: Source::Environment,
            }));
        }
        let path = path()?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("can't read {}", path.display()));
            }
        };
        let source = Source::Credentials(path.clone());
        let key = parse(&text, &source)?;
        warn_if_others_can_read(&path, &source);
        Ok(key.map(|secret| ResendKey { secret, source }))
    }

    /// The Resend API key, or an error listing every way to give one.
    pub fn require() -> Result<Self> {
        match ResendKey::find()? {
            Some(key) => Ok(key),
            None => bail!("{}", missing(&path()?)),
        }
    }
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
