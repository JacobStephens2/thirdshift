//! The User config: `~/.thirdshift/config.toml`, this machine's defaults for
//! every Run. Only a Run, `email-test` and `setup` read it, so a broken one
//! can't block `update`, `version` or `help`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};
use toml_edit::{DocumentMut, Item};

use crate::notification::NotificationAsk;
use crate::run::Goal;

/// The settings a User config can hold. Each is what a Run does when its
/// command says nothing about it.
#[derive(Debug, PartialEq, Eq)]
pub struct UserConfig {
    /// `merge.always`: every Run is a Merge run unless told `no-merge`.
    pub merge_always: bool,
    /// `launch.pull`: every Run fast-forwards the Launch directory's
    /// checkout of the Base branch to `origin`.
    pub launch_pull: bool,
    /// `logs.dir`, with a leading `~` expanded: where session logs are
    /// written, by default `~/.thirdshift/logs`.
    pub logs_dir: PathBuf,
    /// The `[email]` section.
    pub email: EmailSettings,
}

/// The `[email]` section: where email goes and who it comes from. The Resend
/// API key is never here, only in `RESEND_API_KEY`, so the file holds no
/// secret.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmailSettings {
    /// `email.always`: every Run sends a Run notification unless told
    /// `no-email`.
    pub always: bool,
    /// `email.to`: the address email goes to when the command names none.
    pub to: Option<String>,
    /// `email.from`: the sender, in place of Resend's shared test sender.
    pub from: Option<String>,
}

impl UserConfig {
    /// The User config under `$HOME`, or the defaults if there is none.
    pub fn load() -> Result<Self> {
        let (home, path) = home_and_path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => UserConfig::parse(&text, &path, &home),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(UserConfig::defaults(&home))
            }
            Err(error) => Err(error).with_context(|| format!("can't read {}", path.display())),
        }
    }

    /// What a Run does with no User config under `home`.
    fn defaults(home: &Path) -> Self {
        UserConfig {
            merge_always: false,
            launch_pull: false,
            logs_dir: home.join(".thirdshift/logs"),
            email: EmailSettings::default(),
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
        let mut config = UserConfig::defaults(home);
        for (section, value) in &table {
            let known = matches!(section.as_str(), "merge" | "launch" | "logs" | "email");
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
                    ("launch", "pull", Value::Boolean(pull)) => config.launch_pull = *pull,
                    ("launch", "pull", _) => bail!("launch.pull must be true or false in {file}"),
                    ("logs", "dir", Value::String(dir)) => match expand_home(dir, home) {
                        Some(dir) => config.logs_dir = dir,
                        None => bail!(
                            "logs.dir must be an absolute path, ~ or start with ~/, not {dir:?}, in {file}"
                        ),
                    },
                    ("logs", "dir", _) => bail!("logs.dir must be a string in {file}"),
                    ("email", "always", Value::Boolean(always)) => config.email.always = *always,
                    ("email", "always", _) => bail!("email.always must be true or false in {file}"),
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

impl EmailSettings {
    /// What a Run whose command gave no `email` or `no-email` asks about its
    /// Run notification.
    pub fn default_ask(&self) -> NotificationAsk {
        if self.always {
            NotificationAsk::Send(None)
        } else {
            NotificationAsk::Skip
        }
    }
}

/// Setup with no terminal: write the User config with every setting at its
/// default, asking nothing. An existing User config is edited in place, once
/// it parses as a Run would parse it: its values, comments and key order stay,
/// and each key it lacks is added at its default, so Setup never resets a
/// configured machine.
pub fn setup() -> Result<String> {
    let (home, path) = home_and_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(home.join(".thirdshift"))
                .and_then(|()| std::fs::write(&path, DEFAULTS))
                .with_context(|| format!("can't write {}", path.display()))?;
            return Ok(format!("wrote the User config {}", path.display()));
        }
        Err(error) => {
            return Err(error).with_context(|| format!("can't read {}", path.display()));
        }
    };
    UserConfig::parse(&text, &path, &home)?;
    let completed = complete(&text)?;
    if completed == text {
        return Ok(format!(
            "the User config {} already lists every setting",
            path.display()
        ));
    }
    std::fs::write(&path, completed).with_context(|| format!("can't write {}", path.display()))?;
    Ok(format!(
        "added the missing settings to the User config {}",
        path.display()
    ))
}

/// `text`, a User config a Run accepts, with each key it lacks added at its
/// default, as `DEFAULTS` writes it. Everything already in `text` stays as
/// it was: a missing key goes at the end of its section, and a missing
/// section after the last line of `text`.
fn complete(text: &str) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the User config")?;
    let defaults: DocumentMut = DEFAULTS.parse().expect("DEFAULTS is valid TOML");
    let mut missing = DocumentMut::new();
    for (section, default_settings) in defaults.iter() {
        let default_settings = default_settings
            .as_table()
            .expect("every key in DEFAULTS is in a section");
        match document.get_mut(section) {
            None => {
                missing.insert(section, Item::Table(default_settings.clone()));
            }
            Some(Item::Table(settings)) => {
                for (key, item) in default_settings.iter() {
                    if !settings.contains_key(key) {
                        let mut key = default_settings
                            .key(key)
                            .expect("the key is in DEFAULTS")
                            .clone();
                        key.leaf_decor_mut().set_prefix("");
                        settings.insert_formatted(&key, item.clone());
                    }
                }
                if section == "email" && !settings.is_dotted() {
                    note_email_to(settings, default_settings, text);
                }
            }
            Some(Item::Value(toml_edit::Value::InlineTable(settings))) => {
                for (key, item) in default_settings.iter() {
                    if !settings.contains_key(key) {
                        let mut value = item.as_value().expect("DEFAULTS has only values").clone();
                        value.decor_mut().clear();
                        settings.insert(key, value);
                    }
                }
            }
            Some(_) => bail!("{section} must be the section [{section}]"),
        }
    }
    let mut completed = document.to_string();
    let missing = missing.to_string();
    let missing = missing.trim_start_matches('\n');
    if !missing.is_empty() && !completed.trim().is_empty() {
        if !completed.ends_with('\n') {
            completed.push('\n');
        }
        completed.push('\n');
    }
    completed.push_str(missing);
    Ok(completed)
}

/// With no `email.to` in `email`, the `[email]` section of `text`, and no
/// commented-out one either, add the commented-out example line from
/// `defaults`, the `[email]` section of `DEFAULTS`, just before `email.from`:
/// `email.to` has no default to write.
fn note_email_to(email: &mut toml_edit::Table, defaults: &toml_edit::Table, text: &str) {
    if email.contains_key("to") || has_commented_out_email_to(text) {
        return;
    }
    let example = prefix(
        defaults
            .key("from")
            .expect("DEFAULTS has email.from")
            .leaf_decor(),
    );
    let Some(mut from) = email.key_mut("from") else {
        return;
    };
    let decor = from.leaf_decor_mut();
    decor.set_prefix(format!("{example}{}", prefix(decor)));
}

/// The text `decor` puts before a key: the comment and blank lines above it,
/// and its indent.
fn prefix(decor: &toml_edit::Decor) -> String {
    decor
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .unwrap_or("")
        .to_string()
}

/// Whether the `[email]` section of `text` holds a commented-out `to` line.
fn has_commented_out_email_to(text: &str) -> bool {
    let mut in_email = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_email = line
                .trim_start_matches('[')
                .trim_start()
                .starts_with("email")
                && line.trim_end_matches(']').trim_end().ends_with("email");
        } else if in_email && let Some(comment) = line.strip_prefix('#') {
            let comment = comment.trim_start();
            if comment
                .strip_prefix("to")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
            {
                return true;
            }
        }
    }
    false
}

/// The User config Setup writes with no answers: every key at its default,
/// each with what it does and that default. `email.to` has no default, so it
/// is the one key written commented out.
const DEFAULTS: &str = r#"[merge]
always = false   # every Run is a Merge run, without the merge word; default false

[launch]
pull = false     # every Run first fast-forwards your checkout of the Base branch; default false

[email]
always = false                  # every Run sends a Run notification, without the email word; default false
# to = "you@example.com"        # where email goes when the command names no address; no default
from = "onboarding@resend.dev"  # the sender; default onboarding@resend.dev, which only delivers to your Resend account's address

[logs]
dir = "~/.thirdshift/logs"   # where session logs go; default ~/.thirdshift/logs
"#;

/// `$HOME`, and the User config's path under it.
fn home_and_path() -> Result<(PathBuf, PathBuf)> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .context("HOME is not set")?;
    let path = home.join(".thirdshift/config.toml");
    Ok((home, path))
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
    fn the_defaults_setup_writes_are_what_a_run_does_with_no_user_config() {
        let mut config = parse(DEFAULTS).unwrap();
        assert_eq!(
            config.email.from.as_deref(),
            Some(crate::email::DEFAULT_FROM)
        );
        config.email.from = None;
        assert_eq!(config, UserConfig::defaults(Path::new("/home/me")));
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
    fn email_settings_are_read() {
        let config = parse("[email]\nto = \"me@example.com\"\nfrom = \"ts@acme.dev\"\n").unwrap();
        assert_eq!(config.email.to.as_deref(), Some("me@example.com"));
        assert_eq!(config.email.from.as_deref(), Some("ts@acme.dev"));
        assert_eq!(parse("[email]\n").unwrap().email, EmailSettings::default());
        assert!(parse("[email]\nalways = true\n").unwrap().email.always);
        assert!(!parse("[email]\nalways = false\n").unwrap().email.always);
    }

    #[test]
    fn email_always_asks_for_a_notification_to_email_to() {
        let always = parse("[email]\nalways = true\n").unwrap();
        assert_eq!(always.email.default_ask(), NotificationAsk::Send(None));
        for text in ["", "[email]\nalways = false\n"] {
            let config = parse(text).unwrap();
            assert_eq!(
                config.email.default_ask(),
                NotificationAsk::Skip,
                "{text:?}"
            );
        }
    }

    #[test]
    fn logs_dir_is_read_with_a_leading_tilde_expanded_and_defaults_under_home() {
        for (dir, expanded) in [
            ("~/elsewhere/logs", "/home/me/elsewhere/logs"),
            ("~", "/home/me"),
            ("/var/log/thirdshift", "/var/log/thirdshift"),
        ] {
            let config = parse(&format!("[logs]\ndir = {dir:?}\n")).unwrap();
            assert_eq!(config.logs_dir, PathBuf::from(expanded), "{dir}");
        }
        for text in ["", "[logs]\n"] {
            assert_eq!(
                parse(text).unwrap().logs_dir,
                PathBuf::from("/home/me/.thirdshift/logs"),
                "{text:?}"
            );
        }
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
            ("[launch]\npul = true\n", "unknown key launch.pul"),
            ("launch = true\n", "launch must be the section [launch]"),
            (
                "[launch]\npull = \"yes\"\n",
                "launch.pull must be true or false",
            ),
            ("[email]\nadress = \"a@b.c\"\n", "unknown key email.adress"),
            ("email = \"a@b.c\"\n", "email must be the section [email]"),
            (
                "[email]\nalways = \"yes\"\n",
                "email.always must be true or false",
            ),
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

    /// `text` completed, after checking a Run reads it with the settings it
    /// had, plus the defaults for the keys it lacked.
    fn completed(text: &str) -> String {
        let completed = complete(text).unwrap();
        let before = parse(text).unwrap();
        let mut after = parse(&completed).unwrap_or_else(|error| panic!("{error:#}\n{completed}"));
        if before.email.from.is_none() {
            assert_eq!(
                after.email.from.as_deref(),
                Some(crate::email::DEFAULT_FROM)
            );
            after.email.from = None;
        }
        assert_eq!(after, before, "{completed}");
        let again = complete(&completed).unwrap();
        assert_eq!(again, completed, "completing twice changed it");
        completed
    }

    #[test]
    fn completing_an_empty_user_config_writes_the_defaults() {
        assert_eq!(completed(""), DEFAULTS);
    }

    #[test]
    fn completing_the_defaults_changes_nothing() {
        assert_eq!(completed(DEFAULTS), DEFAULTS);
    }

    #[test]
    fn completing_keeps_every_line_already_there_in_order() {
        for text in [
            "# top\n[merge]\nalways = true # mine\n",
            "[merge]\nalways = true",
            "[email]\n# to = \"me@example.com\"\nalways = true\n",
            "[email]\n# a note on from\nfrom = \"ts@acme.dev\"\n",
            "[logs]\ndir = \"/var/log/ts\"\n\n[merge]\nalways = true\n",
            "merge.always = true\nlaunch = { pull = true }\n",
            "[email]\nto = \"me@example.com\"\n",
        ] {
            let completed = completed(text);
            let mut lines = completed.lines();
            for line in text.lines() {
                assert!(
                    lines.any(|kept| kept == line),
                    "{line:?} of {text:?} is lost or moved:\n{completed}"
                );
            }
        }
    }

    #[test]
    fn completing_adds_email_to_commented_out_once() {
        for text in [
            "",
            "[email]\nalways = true\n",
            "[email]\n# a note on from\nfrom = \"ts@acme.dev\"\n",
            "[email]\n# to = \"me@example.com\"\n",
        ] {
            let completed = completed(text);
            let examples = completed
                .lines()
                .filter(|line| line.starts_with("# to = "))
                .count();
            assert_eq!(examples, 1, "{text:?}:\n{completed}");
        }
        let completed = completed("[email]\nto = \"me@example.com\"\n");
        assert!(!completed.contains("# to ="), "{completed}");
    }
}
