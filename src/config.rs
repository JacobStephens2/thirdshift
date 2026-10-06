//! The User config: `~/.thirdshift/config.toml`, this machine's defaults for
//! every Run. Only a Run, `email-test` and `setup` read it, so a broken one
//! can't block `update`, `version` or `help`, and only a Run offers Setup
//! when there is none.

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use toml::{Table, Value};
use toml_edit::{DocumentMut, Item};

use crate::harness::{self, Harness, ModelAndEffort};

/// The settings a User config can hold. Each is what a Run does when its
/// command says nothing about it.
#[derive(Debug, PartialEq, Eq)]
pub struct UserConfig {
    /// `merge.always`: every Run is a Merge run unless told `no-merge`.
    pub merge_always: bool,
    /// `base.fix`: every Run may start a Base fix unless told `no-base-fix`.
    pub base_fix: bool,
    /// `launch.pull`: every Run fast-forwards the Launch directory's
    /// checkout of the Base branch to `origin`.
    pub launch_pull: bool,
    /// `logs.dir`, with a leading `~` expanded: the root of the logs, with
    /// each repository's in `<owner>/<repo>/`, by default
    /// `~/.thirdshift/logs`.
    pub logs_dir: PathBuf,
    /// `activity.quiet_skips`: a skipped Architect run or Pickup run prints
    /// nothing, its Activity log line its only trace.
    pub quiet_skips: bool,
    /// The `[email]` section.
    pub email: EmailSettings,
    /// `spec.parallel`: how many Tickets a Spec run runs at once, by
    /// default 3.
    pub spec_parallel: NonZeroUsize,
    /// `pickup.limit`: the Claim limit, how many open issues carrying a
    /// Claim stop a Pickup run from taking another, by default 3.
    pub pickup_limit: NonZeroUsize,
    /// The `[harness]` section: the Harness every Command runs its sessions
    /// on, and a Model and Effort for each Harness.
    pub harness: harness::Settings,
}

/// The `[email]` section: where email goes and who it comes from. The Resend
/// API key is never here, only in `RESEND_API_KEY` or the Credentials, so the
/// file holds no secret.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmailSettings {
    /// `email.always`: every Run, and every Architect run, sends a Run
    /// notification unless told `no-email`.
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
            base_fix: false,
            launch_pull: false,
            logs_dir: home.join(".thirdshift/logs"),
            quiet_skips: false,
            email: EmailSettings::default(),
            spec_parallel: NonZeroUsize::new(3).unwrap(),
            pickup_limit: NonZeroUsize::new(3).unwrap(),
            harness: harness::Settings::default(),
        }
    }

    /// Parse `text`, the contents of the User config at `path`, expanding a
    /// leading `~` to `home`. Any key or section thirdshift doesn't know is an
    /// error, so a typo can't silently leave a setting unset.
    pub fn parse(text: &str, path: &Path, home: &Path) -> Result<Self> {
        let file = format!("the User config {}", path.display());
        let table: Table = text
            .parse()
            .map_err(|error| anyhow!("{error}"))
            .with_context(|| format!("can't parse {file}"))?;
        let mut config = UserConfig::defaults(home);
        for (section, value) in &table {
            let known = matches!(
                section.as_str(),
                "merge"
                    | "base"
                    | "launch"
                    | "logs"
                    | "activity"
                    | "email"
                    | "spec"
                    | "pickup"
                    | "harness"
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
                    ("base", "fix", Value::Boolean(fix)) => config.base_fix = *fix,
                    ("base", "fix", _) => bail!("base.fix must be true or false in {file}"),
                    ("launch", "pull", Value::Boolean(pull)) => config.launch_pull = *pull,
                    ("launch", "pull", _) => bail!("launch.pull must be true or false in {file}"),
                    ("logs", "dir", Value::String(dir)) => match expand_home(dir, home) {
                        Some(dir) => config.logs_dir = dir,
                        None => bail!(
                            "logs.dir must be an absolute path, ~ or start with ~/, not {dir:?}, in {file}"
                        ),
                    },
                    ("logs", "dir", _) => bail!("logs.dir must be a string in {file}"),
                    ("activity", "quiet_skips", Value::Boolean(quiet)) => {
                        config.quiet_skips = *quiet
                    }
                    ("activity", "quiet_skips", _) => {
                        bail!("activity.quiet_skips must be true or false in {file}")
                    }
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
                        config.spec_parallel = whole_number_from_1(value, "spec.parallel", &file)?
                    }
                    ("pickup", "limit", value) => {
                        config.pickup_limit = whole_number_from_1(value, "pickup.limit", &file)?
                    }
                    ("harness", "default", Value::String(name)) => match Harness::named(name) {
                        Some(harness) => config.harness.default = Some(harness),
                        None => bail!(
                            "harness.default must be {}, not {name:?}, in {file}",
                            harness::names()
                        ),
                    },
                    ("harness", "default", _) => {
                        bail!("harness.default must be a quoted name in {file}")
                    }
                    ("harness", name, value) if Harness::named(name).is_some() => {
                        let Value::Table(settings) = value else {
                            bail!("harness.{key} must be the section [harness.{key}] in {file}")
                        };
                        let harness = Harness::named(name).expect("a Harness's name");
                        *config.harness.of_mut(harness) = model_and_effort(settings, key, &file)?;
                    }
                    _ => bail!("unknown key {section}.{key} in {file}"),
                }
            }
        }
        Ok(config)
    }
}

/// Replace the file at `path` with `text` all at once, keeping its
/// permissions, so a failed write leaves it as it was.
pub fn replace(path: &Path, text: &str) -> std::io::Result<()> {
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
/// section, or a key of a section `text` names only in its subsections'
/// headers, after the last line of `text`. A key added to an inline table
/// gets no comment, as TOML has no place for one there.
pub fn complete(text: &str) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the User config")?;
    let defaults: DocumentMut = DEFAULTS.parse().expect("DEFAULTS is valid TOML");
    let mut missing = DocumentMut::new();
    complete_table(
        document.as_table_mut(),
        defaults.as_table(),
        missing.as_table_mut(),
    );
    if let Some(Item::Table(email)) = document.get_mut("email")
        && !email.is_dotted()
    {
        let defaults = defaults["email"].as_table().expect("DEFAULTS has [email]");
        note_email_to(email, defaults, text);
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

/// Add to `settings`, a table of a User config, each key of `defaults`, the
/// same table in `DEFAULTS`, that it lacks, and the same for each of its
/// subsections. A missing key goes at the end of `settings`, unless
/// `settings` has no header of its own, as `[harness]` with only
/// `[harness.claude]` written; then it goes, as does a missing subsection, in
/// `missing`, the same table of what is written after the file.
fn complete_table(
    settings: &mut toml_edit::Table,
    defaults: &toml_edit::Table,
    missing: &mut toml_edit::Table,
) {
    let implicit = settings.is_implicit() && !settings.is_dotted();
    for (key, item) in defaults.iter() {
        let default_key = defaults.key(key).expect("the key is in DEFAULTS");
        match (item, settings.get_mut(key)) {
            (Item::Table(default_settings), Some(Item::Table(settings))) => {
                let missing = missing.entry(key).or_insert_with(|| {
                    let mut table = toml_edit::Table::new();
                    table.set_implicit(true);
                    table.set_position(default_settings.position());
                    Item::Table(table)
                });
                let missing = missing
                    .as_table_mut()
                    .expect("a missing section is a table");
                complete_table(settings, default_settings, missing);
            }
            (
                Item::Table(default_settings),
                Some(Item::Value(toml_edit::Value::InlineTable(settings))),
            ) => complete_inline(settings, default_settings),
            (Item::Table(_), Some(_)) => unreachable!("a Run refuses {key} that isn't a section"),
            (Item::Table(_), None) => {
                missing.insert_formatted(default_key, item.clone());
            }
            (_, Some(_)) => {}
            (_, None) if implicit => {
                missing.set_implicit(false);
                missing.insert_formatted(default_key, item.clone());
            }
            (_, None) => {
                let mut key = default_key.clone();
                key.leaf_decor_mut().set_prefix("");
                settings.insert_formatted(&key, item.clone());
            }
        }
    }
}

/// Add to `settings`, an inline table of a User config, each key of
/// `defaults`, the same table in `DEFAULTS`, that it lacks, with no comment.
fn complete_inline(settings: &mut toml_edit::InlineTable, defaults: &toml_edit::Table) {
    for (key, item) in defaults.iter() {
        match (item, settings.get_mut(key)) {
            (Item::Table(defaults), Some(toml_edit::Value::InlineTable(settings))) => {
                complete_inline(settings, defaults)
            }
            (_, Some(_)) => {}
            (Item::Table(defaults), None) => {
                let mut table = toml_edit::InlineTable::new();
                complete_inline(&mut table, defaults);
                settings.insert(key, toml_edit::Value::InlineTable(table));
            }
            (_, None) => {
                let mut value = item.as_value().expect("DEFAULTS has only values").clone();
                value.decor_mut().clear();
                settings.insert(key, value);
            }
        }
    }
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
pub fn decor_prefix(decor: &toml_edit::Decor) -> String {
    decor
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .unwrap_or("")
        .to_string()
}

/// The text `decor` puts after a value: the spaces and comment that end its
/// line.
pub fn decor_suffix(decor: &toml_edit::Decor) -> String {
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
pub fn is_commented_out_email_to(line: &str) -> bool {
    line.trim()
        .strip_prefix('#')
        .and_then(|comment| comment.trim_start().strip_prefix("to"))
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// The User config Setup writes with no answers: every key at its default,
/// each with what it does and that default. `email.to` has no default, so it
/// is the one key written commented out.
pub const DEFAULTS: &str = r#"[merge]
always = false   # every Run is a Merge run, without the merge word; default false

[base]
fix = false      # every Run may start a Base fix, without the base-fix word; default false

[launch]
pull = false     # every Run first fast-forwards your checkout of the Base branch; default false

[email]
always = false                  # every Run sends a Run notification, without the email word; default false
# to = "you@example.com"        # where email goes when the command names no address; no default
from = "onboarding@resend.dev"  # the sender; default onboarding@resend.dev, which only delivers to your Resend account's address

[logs]
dir = "~/.thirdshift/logs"   # the root of the logs, each repository's in <owner>/<repo>/; default ~/.thirdshift/logs

[activity]
quiet_skips = false   # a skipped Architect run or Pickup run prints nothing, leaving only its Activity log line; default false

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3

[pickup]
limit = 3   # how many open issues labelled in-progress stop a Pickup run taking another; default 3

[harness]
default = "claude"   # the Harness every Run's sessions run on, claude or codex; default claude

[harness.claude]
model = ""    # the Model Claude Code's sessions run on; default blank, for Claude Code's own
effort = ""   # how hard that Model reasons; default blank, for Claude Code's own

[harness.codex]
model = ""    # the Model Codex's sessions run on; default blank, for Codex's own
effort = ""   # how hard that Model reasons; default blank, for Codex's own
"#;

/// The line `DEFAULTS` holds for `email.to`, which has no default.
const NO_EMAIL_TO: &str = r#"# to = "you@example.com"        # where email goes when the command names no address; no default"#;

/// `DEFAULTS` with `email.to` set to `to`, if there is one.
pub fn with_email_to(to: Option<String>) -> String {
    let Some(to) = to else {
        return DEFAULTS.to_string();
    };
    let (_, comment) = NO_EMAIL_TO.split_once("  # ").unwrap();
    let line = format!("to = {}", Value::String(to));
    DEFAULTS.replace(NO_EMAIL_TO, &format!("{line:<31} # {comment}"))
}

/// `value`, which `file` gives the setting `key`, as a whole number from 1
/// up, or an error naming both if it is anything else.
fn whole_number_from_1(value: &Value, key: &str, file: &str) -> Result<NonZeroUsize> {
    value
        .as_integer()
        .and_then(|n| usize::try_from(n).ok())
        .and_then(NonZeroUsize::new)
        .with_context(|| format!("{key} must be a whole number from 1 up in {file}"))
}

/// `settings`, the section `[harness.<harness>]` of `file`, as the Model and
/// Effort set for that Harness, each blank to leave it to the Harness. Any
/// other key, or a value that isn't a string, is an error naming it.
fn model_and_effort(settings: &Table, harness: &str, file: &str) -> Result<ModelAndEffort> {
    let mut set = ModelAndEffort::default();
    for (key, value) in settings {
        let setting = match key.as_str() {
            "model" => &mut set.model,
            "effort" => &mut set.effort,
            _ => bail!("unknown key harness.{harness}.{key} in {file}"),
        };
        let Value::String(value) = value else {
            bail!(
                "harness.{harness}.{key} must be a quoted string, blank for {harness}'s own default, in {file}"
            );
        };
        *setting = Some(value.clone()).filter(|value| !value.is_empty());
    }
    Ok(set)
}

/// `$HOME`, and the User config's path under it.
pub fn home_and_path() -> Result<(PathBuf, PathBuf)> {
    let home = home()?;
    let path = home.join(".thirdshift/config.toml");
    Ok((home, path))
}

/// The user's home folder, `$HOME`, where `~/.thirdshift` is.
pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .context("HOME is not set")
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
        assert_eq!(config.harness.default, Some(Harness::Claude));
        config.harness.default = None;
        assert_eq!(config, UserConfig::defaults(Path::new("/home/me")));
    }

    #[test]
    fn every_default_key_has_a_comment_giving_what_it_does_and_its_default() {
        let mut section = "";
        let mut keys = Vec::new();
        for line in DEFAULTS.lines().filter(|line| !line.is_empty()) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name;
                continue;
            }
            let (setting, comment) = line.split_once(" # ").unwrap_or_else(|| {
                panic!("no trailing comment on [{section}] {line:?}");
            });
            assert!(comment.contains("default"), "[{section}] {line:?}");
            let key = setting.trim_start_matches("# ").split(' ').next().unwrap();
            keys.push(format!("{section}.{key}"));
        }
        assert_eq!(
            keys,
            [
                "merge.always",
                "base.fix",
                "launch.pull",
                "email.always",
                "email.to",
                "email.from",
                "logs.dir",
                "activity.quiet_skips",
                "spec.parallel",
                "pickup.limit",
                "harness.default",
                "harness.claude.model",
                "harness.claude.effort",
                "harness.codex.model",
                "harness.codex.effort"
            ]
        );
        let commented_out: Vec<&str> = DEFAULTS
            .lines()
            .filter(|line| line.starts_with('#'))
            .collect();
        assert_eq!(commented_out, [NO_EMAIL_TO]);
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
    fn the_harness_section_sets_the_default_harness_and_a_model_and_effort_for_each() {
        let config = parse(
            "[harness]\ndefault = \"codex\"\n\n\
             [harness.claude]\nmodel = \"opus\"\neffort = \"\"\n\n\
             [harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"max\"\n",
        )
        .unwrap();

        assert_eq!(
            config.harness,
            harness::Settings {
                default: Some(Harness::Codex),
                claude: ModelAndEffort {
                    model: Some("opus".to_string()),
                    effort: None,
                },
                codex: ModelAndEffort {
                    model: Some("gpt-6.1-sol".to_string()),
                    effort: Some("max".to_string()),
                },
            }
        );
        assert_eq!(parse("").unwrap().harness, harness::Settings::default());
    }

    #[test]
    fn a_harness_setting_thirdshift_cant_use_is_an_error_naming_it() {
        for (text, error) in [
            (
                "[harness]\ndefault = \"gemini\"\n",
                "harness.default must be claude or codex, not \"gemini\"",
            ),
            (
                "[harness]\ndefault = true\n",
                "harness.default must be a quoted name",
            ),
            ("[harness]\nmodel = \"opus\"\n", "unknown key harness.model"),
            (
                "[harness.gemini]\nmodel = \"x\"\n",
                "unknown key harness.gemini",
            ),
            (
                "[harness]\nclaude = \"opus\"\n",
                "harness.claude must be the section [harness.claude]",
            ),
            (
                "[harness.claude]\nmodels = \"opus\"\n",
                "unknown key harness.claude.models",
            ),
            (
                "[harness.codex]\neffort = 3\n",
                "harness.codex.effort must be a quoted string",
            ),
            (
                "harness = \"claude\"\n",
                "harness must be the section [harness]",
            ),
        ] {
            let error_text = format!("{:#}", parse(text).unwrap_err());

            assert!(error_text.contains(error), "{text:?}: {error_text}");
        }
    }

    #[test]
    fn merge_always_is_read() {
        assert!(parse("[merge]\nalways = true\n").unwrap().merge_always);
        assert!(!parse("[merge]\nalways = false\n").unwrap().merge_always);
        assert!(!parse("").unwrap().merge_always);
    }

    #[test]
    fn base_fix_is_read() {
        assert!(parse("[base]\nfix = true\n").unwrap().base_fix);
        assert!(!parse("[base]\nfix = false\n").unwrap().base_fix);
        assert!(!parse("").unwrap().base_fix);
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
    fn pickup_limit_is_read_and_defaults_to_3() {
        let config = parse("[pickup]\nlimit = 1\n").unwrap();
        assert_eq!(config.pickup_limit.get(), 1);
        for text in ["", "[pickup]\n"] {
            assert_eq!(parse(text).unwrap().pickup_limit.get(), 3, "{text:?}");
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
    fn activity_quiet_skips_is_read_and_defaults_to_false() {
        assert!(
            parse("[activity]\nquiet_skips = true\n")
                .unwrap()
                .quiet_skips
        );
        for text in ["", "[activity]\n", "[activity]\nquiet_skips = false\n"] {
            assert!(!parse(text).unwrap().quiet_skips, "{text:?}");
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
            ("[base]\nfx = true\n", "unknown key base.fx"),
            ("base = true\n", "base must be the section [base]"),
            ("[base]\nfix = \"yes\"\n", "base.fix must be true or false"),
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
            ("[pickup]\nlimt = 2\n", "unknown key pickup.limt"),
            ("pickup = 2\n", "pickup must be the section [pickup]"),
            (
                "[pickup]\nlimit = 0\n",
                "pickup.limit must be a whole number from 1 up",
            ),
            (
                "[pickup]\nlimit = -1\n",
                "pickup.limit must be a whole number from 1 up",
            ),
            (
                "[pickup]\nlimit = 2.5\n",
                "pickup.limit must be a whole number from 1 up",
            ),
            (
                "[pickup]\nlimit = \"2\"\n",
                "pickup.limit must be a whole number from 1 up",
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
    /// had, plus the defaults for the keys it lacked, which change nothing a
    /// Run does.
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
        if before.harness.default.is_none() {
            assert_eq!(after.harness.default, Some(Harness::Claude));
            after.harness.default = None;
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
    fn completing_adds_each_harness_key_a_user_config_lacks() {
        for text in [
            "[harness]\ndefault = \"claude\"\n",
            "[harness.claude]\nmodel = \"opus\" # mine\n",
            "[harness.codex]\neffort = \"max\"\n\n[merge]\nalways = true\n",
            "[harness]\n\n[harness.claude]\neffort = \"high\"\n",
            "harness.default = \"claude\"\nharness.claude.model = \"opus\"\n",
            "harness = { claude = { model = \"opus\" } }\n",
            "[harness]\nclaude = { effort = \"low\" }\n",
        ] {
            let completed = completed(text);
            let config: toml::Table = completed.parse().unwrap();
            let harness = config["harness"].as_table().unwrap();
            assert!(harness.contains_key("default"), "{text:?}:\n{completed}");
            for name in ["claude", "codex"] {
                let settings = harness[name].as_table().unwrap();
                for key in ["model", "effort"] {
                    assert!(settings.contains_key(key), "{text:?}:\n{completed}");
                }
            }
            // An inline table gains its missing keys on its own line.
            let mut lines = completed.lines();
            for line in text.lines().filter(|line| !line.contains('{')) {
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
}
