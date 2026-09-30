//! The User config: `~/.thirdshift/config.toml`, this machine's defaults for
//! every Run. Only a Run, `email-test` and `setup` read it, so a broken one
//! can't block `update`, `version` or `help`.

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};
use toml_edit::{DocumentMut, Item};

use crate::git::Git;
use crate::notification::NotificationAsk;
use crate::questions::{self, Answers};
use crate::run::Goal;
use crate::{email, github, progress};

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
    /// `spec.parallel`: how many Tickets a Spec run runs at once, by
    /// default 3.
    pub spec_parallel: NonZeroUsize,
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
            spec_parallel: NonZeroUsize::new(3).unwrap(),
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
            let known = matches!(
                section.as_str(),
                "merge" | "launch" | "logs" | "email" | "spec"
            );
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
                    ("spec", "parallel", value) => {
                        let parallel = value
                            .as_integer()
                            .and_then(|n| usize::try_from(n).ok())
                            .and_then(NonZeroUsize::new);
                        match parallel {
                            Some(parallel) => config.spec_parallel = parallel,
                            None => {
                                bail!("spec.parallel must be a whole number from 1 up in {file}")
                            }
                        }
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

/// Setup: write the User config. From a terminal, the Setup questions come
/// first, each with the current value as its default answer, and the answers
/// are written; with no terminal, nothing is asked, and every setting is at
/// its default, with `email.to` as the suggested address, if there is one.
/// An existing User config is edited in place, once it parses as a Run would
/// parse it: its comments and key order stay, as do the values Setup didn't
/// ask about, and each key it lacks is added at its default, so Setup with no
/// terminal never resets a configured machine. Nothing is written until the
/// last answer is in. A test email, if the user asked for one, goes once the
/// User config is written, as `email-test` sends it.
pub fn setup() -> Result<String> {
    let (home, path) = home_and_path()?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("can't read {}", path.display()));
        }
    };
    let mut text = match &existing {
        Some(text) => {
            UserConfig::parse(text, &path, &home)?;
            complete(text)?
        }
        None => with_email_to(suggested_address(&home)),
    };
    let asked = questions::has_terminal();
    let mut send_test = false;
    if asked {
        let current = UserConfig::parse(&text, &path, &home)?;
        let answers = questions::ask(&current, || suggested_address(&home))?;
        send_test = answers.send_test();
        text = with_answers(&text, &answers)?;
    }
    let written = match &existing {
        None => {
            std::fs::create_dir_all(home.join(".thirdshift"))
                .and_then(|()| std::fs::write(&path, &text))
                .with_context(|| format!("can't write {}", path.display()))?;
            format!("wrote the User config {}", path.display())
        }
        Some(existing) if *existing == text && asked => {
            format!(
                "the User config {} already holds your answers",
                path.display()
            )
        }
        Some(existing) if *existing == text => format!(
            "the User config {} already lists every setting",
            path.display()
        ),
        Some(_) => {
            replace(&path, &text).with_context(|| format!("can't write {}", path.display()))?;
            if asked {
                format!("wrote your answers to the User config {}", path.display())
            } else {
                format!(
                    "added the missing settings to the User config {}",
                    path.display()
                )
            }
        }
    };
    if !send_test {
        return Ok(written);
    }
    progress::step(written);
    let config = UserConfig::load()?;
    Ok(email::send_test(None, &config.email)?.to_string())
}

/// `text`, a User config with every key, or a commented-out `email.to`, with
/// the values `answers` gives. Only those values change: the spacing and
/// comments around each stay, as does everything else in `text`.
fn with_answers(text: &str, answers: &Answers) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the User config")?;
    set(&mut document, "merge", "always", answers.merge_always);
    set(&mut document, "launch", "pull", answers.launch_pull);
    set(
        &mut document,
        "email",
        "always",
        answers.notifications.is_some(),
    );
    if let Some(notifications) = &answers.notifications {
        set(&mut document, "email", "from", notifications.from.as_str());
        set_email_to(&mut document, &notifications.to);
    }
    Ok(document.to_string())
}

/// Set `section.key`, which `document` holds, to `value`, keeping the
/// spacing and comment around the old value, and the comment in the same
/// column where the spaces before it allow. An equal value is left as it was
/// written.
fn set(document: &mut DocumentMut, section: &str, key: &str, value: impl Into<toml_edit::Value>) {
    let value = value.into();
    let old = document[section]
        .as_table_like_mut()
        .and_then(|settings| settings.get_mut(key))
        .and_then(Item::as_value_mut)
        .unwrap_or_else(|| panic!("a completed User config has {section}.{key}"));
    let same = match (old.as_bool(), old.as_str()) {
        (Some(old), _) => value.as_bool() == Some(old),
        (_, Some(old)) => value.as_str() == Some(old),
        _ => false,
    };
    if same {
        return;
    }
    let mut decor = old.decor().clone();
    let suffix = decor_suffix(&decor);
    let spaces = suffix.len() - suffix.trim_start_matches(' ').len();
    if spaces > 0 && suffix[spaces..].starts_with('#') {
        let width = |value: &toml_edit::Value| value.clone().decorated("", "").to_string().len();
        let spaces = (spaces + width(old)).saturating_sub(width(&value)).max(1);
        decor.set_suffix(format!(
            "{}{}",
            " ".repeat(spaces),
            suffix.trim_start_matches(' ')
        ));
    }
    *old = value;
    *old.decor_mut() = decor;
}

/// Set `email.to` in `document` to `to`. With no `email.to` there yet, it
/// takes the place of the commented-out one, if there is one, written as
/// `DEFAULTS` would write it, with its comment.
fn set_email_to(document: &mut DocumentMut, to: &str) {
    let email = &mut document["email"];
    let has_to = email
        .as_table_like()
        .is_some_and(|settings| settings.contains_key("to"));
    if has_to {
        set(document, "email", "to", to);
        return;
    }
    let Some(email) = email.as_table_mut().filter(|email| !email.is_dotted()) else {
        let settings = email.as_table_like_mut().expect("[email] is a section");
        settings.insert("to", Item::Value(to.into()));
        return;
    };
    let example: DocumentMut = with_email_to(Some(to.to_string()))
        .parse()
        .expect("DEFAULTS is valid TOML");
    let (key, item) = example["email"]
        .as_table()
        .and_then(|example| example.get_key_value("to"))
        .expect("the example sets email.to");
    let mut key = key.clone();
    key.leaf_decor_mut().set_prefix("");
    let keys: Vec<String> = email.iter().map(|(key, _)| key.to_string()).collect();
    let mut place = keys.len();
    for (at, name) in keys.iter().enumerate() {
        let mut next = email.key_mut(name).expect("the key is in [email]");
        let prefix = decor_prefix(next.leaf_decor());
        let mut end = 0;
        let found = prefix.split_inclusive('\n').find_map(|line| {
            let start = end;
            end += line.len();
            is_commented_out_email_to(line).then_some((start, end))
        });
        let Some((start, end)) = found else {
            continue;
        };
        next.leaf_decor_mut().set_prefix(&prefix[end..]);
        key.leaf_decor_mut().set_prefix(&prefix[..start]);
        place = at;
        break;
    }
    email.insert_formatted(&key, item.clone());
    // Each key keeps its place, and `to` goes just before the key at `place`,
    // or last: odd ranks for the keys that were there, an even one for `to`.
    let rank = |name: &str| match keys.iter().position(|key| key == name) {
        Some(at) => 2 * at + 1,
        None => 2 * place,
    };
    email.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
}

/// Replace the file at `path` with `text` all at once, keeping its
/// permissions, so a failed write leaves it as it was.
fn replace(path: &Path, text: &str) -> std::io::Result<()> {
    let permissions = std::fs::metadata(path)?.permissions();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.as_file().set_permissions(permissions)?;
    file.persist(path)?;
    Ok(())
}

/// `text`, a User config a Run accepts, with each key it lacks added at its
/// default, as `DEFAULTS` writes it. Everything already in `text` stays as
/// it was: a missing key goes at the end of its section, and a missing
/// section after the last line of `text`. A key added to an inline table
/// gets no comment, as TOML has no place for one there.
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
            Some(_) => unreachable!("a Run refuses {section} that isn't a section"),
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
    let example = decor_prefix(
        defaults
            .key("from")
            .expect("DEFAULTS has email.from")
            .leaf_decor(),
    );
    let Some(mut from) = email.key_mut("from") else {
        return;
    };
    let decor = from.leaf_decor_mut();
    decor.set_prefix(format!("{example}{}", decor_prefix(decor)));
}

/// The text `decor` puts before a key: the comment and blank lines above it,
/// and its indent.
fn decor_prefix(decor: &toml_edit::Decor) -> String {
    decor
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .unwrap_or("")
        .to_string()
}

/// The text `decor` puts after a value: the spaces and comment that end its
/// line.
fn decor_suffix(decor: &toml_edit::Decor) -> String {
    decor
        .suffix()
        .and_then(|suffix| suffix.as_str())
        .unwrap_or("")
        .to_string()
}

/// Whether the `[email]` section of `text` holds a commented-out `to` line.
fn has_commented_out_email_to(text: &str) -> bool {
    let mut in_email = false;
    for line in text.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let header = header.split('#').next().unwrap_or("").trim_end();
            in_email = header.strip_suffix(']').map(str::trim) == Some("email");
        } else if in_email && is_commented_out_email_to(line) {
            return true;
        }
    }
    false
}

/// Whether `line`, in the `[email]` section, is a commented-out `to` line.
fn is_commented_out_email_to(line: &str) -> bool {
    line.trim()
        .strip_prefix('#')
        .and_then(|comment| comment.trim_start().strip_prefix("to"))
        .is_some_and(|rest| rest.trim_start().starts_with('='))
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

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3
"#;

/// The line `DEFAULTS` holds for `email.to`, which has no default.
const NO_EMAIL_TO: &str = r#"# to = "you@example.com"        # where email goes when the command names no address; no default"#;

/// `DEFAULTS` with `email.to` set to `to`, if there is one.
fn with_email_to(to: Option<String>) -> String {
    let Some(to) = to else {
        return DEFAULTS.to_string();
    };
    let (_, comment) = NO_EMAIL_TO.split_once("  # ").unwrap();
    let line = format!("to = {}", Value::String(to));
    DEFAULTS.replace(NO_EMAIL_TO, &format!("{line:<31} # {comment}"))
}

/// The address Setup suggests for `email.to`: the public email of the user's
/// GitHub profile, else the global git `user.email`, unless that is a
/// `@users.noreply.github.com` address, which can't receive mail. Only Setup
/// looks it up; a Run never falls back to it.
fn suggested_address(home: &Path) -> Option<String> {
    let github = github::profile_email().ok().flatten();
    github.filter(|email| !email.is_empty()).or_else(|| {
        let git = Git::new(home).run(&["config", "--global", "user.email"]);
        git.ok().filter(|email| {
            !email.is_empty()
                && !email
                    .to_ascii_lowercase()
                    .ends_with("@users.noreply.github.com")
        })
    })
}

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
    fn a_suggested_address_sets_email_to_in_place_of_the_commented_out_line() {
        assert!(DEFAULTS.contains(&format!("\n{NO_EMAIL_TO}\n")));
        assert_eq!(with_email_to(None), DEFAULTS);
        let text = with_email_to(Some("o\"brien@example.com".to_string()));
        let config = parse(&text).unwrap();
        assert_eq!(config.email.to.as_deref(), Some("o\"brien@example.com"));
        let line = text.lines().find(|line| line.starts_with("to = ")).unwrap();
        let from = text
            .lines()
            .find(|line| line.starts_with("from = "))
            .unwrap();
        assert_eq!(
            line.find(" # "),
            from.find("  # ").map(|at| at + 1),
            "{text}"
        );
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
    fn spec_parallel_is_read_and_defaults_to_3() {
        let config = parse("[spec]\nparallel = 5\n").unwrap();
        assert_eq!(config.spec_parallel.get(), 5);
        for text in ["", "[spec]\n"] {
            assert_eq!(parse(text).unwrap().spec_parallel.get(), 3, "{text:?}");
        }
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
            ("[spec]\nparalel = 2\n", "unknown key spec.paralel"),
            ("spec = 2\n", "spec must be the section [spec]"),
            (
                "[spec]\nparallel = 0\n",
                "spec.parallel must be a whole number from 1 up",
            ),
            (
                "[spec]\nparallel = -1\n",
                "spec.parallel must be a whole number from 1 up",
            ),
            (
                "[spec]\nparallel = 2.5\n",
                "spec.parallel must be a whole number from 1 up",
            ),
            (
                "[spec]\nparallel = \"2\"\n",
                "spec.parallel must be a whole number from 1 up",
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
            "[ email ]  # mine\n# to = \"me@example.com\"\n",
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

    fn notifications_to(to: &str) -> Answers {
        Answers {
            merge_always: false,
            launch_pull: false,
            notifications: Some(questions::Notifications {
                to: to.to_string(),
                from: crate::email::DEFAULT_FROM.to_string(),
                send_test: false,
            }),
        }
    }

    #[test]
    fn an_answered_address_takes_the_place_of_the_commented_out_line() {
        let answered = with_answers(DEFAULTS, &notifications_to("me@example.com")).unwrap();
        let expected = with_email_to(Some("me@example.com".to_string())).replace(
            "always = false                  #",
            "always = true                   #",
        );
        assert_eq!(answered, expected);
    }

    #[test]
    fn an_answered_address_keeps_the_lines_around_the_commented_out_one() {
        let text =
            "[email]\nalways = false\n\n# mine\n# to = \"x@y.z\"\n# more\nfrom = \"a@b.c\"\n";
        let answered = with_answers(
            &complete(text).unwrap(),
            &notifications_to("me@example.com"),
        )
        .unwrap();
        let lines: Vec<&str> = answered.lines().collect();
        assert_eq!(
            lines[..4],
            ["[email]", "always = true", "", "# mine"],
            "{answered}"
        );
        assert!(
            lines[4].starts_with("to = \"me@example.com\""),
            "{answered}"
        );
        assert_eq!(
            lines[5..7],
            ["# more", "from = \"onboarding@resend.dev\""],
            "{answered}"
        );
    }
}
