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
    /// `activity.quiet_skips`: a skipped Pass prints
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

/// A validated User config with every setting present, ready for Setup.
/// Preparation and edits preserve the user's TOML spelling and comments.
pub struct UserConfigDocument {
    document: DocumentMut,
    settings: UserConfig,
}

/// The Setup settings to change. Other settings keep their current values.
/// Credentials and the decision to send a test email stay with Setup.
pub struct UserConfigChanges {
    pub merge_always: bool,
    pub base_fix: bool,
    pub launch_pull: bool,
    /// None disables Run notifications while retaining their addresses.
    pub notifications: Option<NotificationAddresses>,
    /// None retains every Harness setting. Unset Model/Effort writes blank.
    pub harness: Option<(Harness, ModelAndEffort)>,
}

/// Recipient and sender for enabled Run notifications.
pub struct NotificationAddresses {
    pub to: String,
    pub from: String,
}

impl UserConfigDocument {
    /// Prepare existing text at `path`, rejecting anything a Run rejects
    /// before adding defaults. Paths in the completed settings use `home`.
    pub fn existing(text: &str, path: &Path, home: &Path) -> Result<Self> {
        Self::prepare(text, path, home)
    }

    /// Prepare a new User config at its defaults, with a suggested recipient
    /// if Setup found one. This never reads or writes a file.
    pub fn defaults(path: &Path, home: &Path, suggested_to: Option<String>) -> Result<Self> {
        Self::prepare(&with_email_to(suggested_to), path, home)
    }

    fn prepare(text: &str, path: &Path, home: &Path) -> Result<Self> {
        UserConfig::parse(text, path, home)?;
        let completed = complete(text)?;
        let settings = UserConfig::parse(&completed, path, home)?;
        // Completion appends missing implicit sections after the original
        // text. Reparse that text so subsequent edits keep their placement.
        let document = completed.parse().context("can't parse the User config")?;
        Ok(Self { document, settings })
    }

    /// Completed settings, used as Setup's question defaults.
    pub fn settings(&self) -> &UserConfig {
        &self.settings
    }

    /// Return the completed text, applying only the supplied semantic
    /// changes. Consumes preparation so its settings cannot become stale.
    pub fn render(self, changes: Option<UserConfigChanges>) -> String {
        let mut document = self.document;
        if let Some(changes) = changes {
            set(&mut document, "merge", "always", changes.merge_always);
            set(&mut document, "base", "fix", changes.base_fix);
            set(&mut document, "launch", "pull", changes.launch_pull);
            set(
                &mut document,
                "email",
                "always",
                changes.notifications.is_some(),
            );
            if let Some(notifications) = &changes.notifications {
                set(&mut document, "email", "from", notifications.from.as_str());
                set_email_to(&mut document, &notifications.to);
            }
            if let Some((harness, chosen)) = &changes.harness {
                set(&mut document, "harness", "default", harness.name());
                let section = format!("harness.{}", harness.name());
                for (key, value) in [("model", &chosen.model), ("effort", &chosen.effort)] {
                    set(&mut document, &section, key, value.as_deref().unwrap_or(""));
                }
            }
        }
        document.to_string()
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
fn complete(text: &str) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the User config")?;
    let has_email_example = email_example(&document).is_some();
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
        note_email_to(email, defaults, has_email_example);
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

/// Set `section.key`, which `document` holds, to `value`, keeping the
/// spacing and comment around the old value, and the comment in the same
/// column where the spaces before it allow. An equal value is left as it was
/// written. `section` may be a subsection, as `harness.claude`.
fn set(document: &mut DocumentMut, section: &str, key: &str, value: impl Into<toml_edit::Value>) {
    let value = value.into();
    let old = section
        .split('.')
        .try_fold(document.as_item_mut(), |item, name| {
            item.as_table_like_mut()?.get_mut(name)
        })
        .and_then(Item::as_table_like_mut)
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
/// the defaults would write it, with its comment.
fn set_email_to(document: &mut DocumentMut, to: &str) {
    let email = &mut document["email"];
    let has_to = email
        .as_table_like()
        .is_some_and(|settings| settings.contains_key("to"));
    if has_to {
        set(document, "email", "to", to);
        return;
    }
    if email.as_table().is_none_or(|email| email.is_dotted()) {
        let settings = email.as_table_like_mut().expect("[email] is a section");
        settings.insert("to", Item::Value(to.into()));
        return;
    }
    let example: DocumentMut = with_email_to(Some(to.to_string()))
        .parse()
        .expect("DEFAULTS is valid TOML");
    let (key, item) = example["email"]
        .as_table()
        .and_then(|example| example.get_key_value("to"))
        .expect("the example sets email.to");
    let mut key = key.clone();
    key.leaf_decor_mut().set_prefix("");
    let email = document["email"].as_table().expect("[email] is a section");
    let keys: Vec<String> = email.iter().map(|(key, _)| key.to_string()).collect();
    let mut place = keys.len();
    let prefix = match email_example(document) {
        Some(EmailExample::BeforeKey(name)) => {
            place = keys
                .iter()
                .position(|key| *key == name)
                .expect("the key is in [email]");
            let mut next = document["email"]
                .as_table_mut()
                .unwrap()
                .key_mut(&name)
                .unwrap();
            take_email_example(next.leaf_decor_mut())
        }
        Some(EmailExample::BeforeSection(position)) => {
            let next = section_at(document.as_table_mut(), position)
                .expect("the section is in the document");
            take_email_example(next.decor_mut())
        }
        Some(EmailExample::Trailing) => {
            let prefix = document.trailing().as_str().unwrap_or("");
            let (before, after) = split_email_example(prefix);
            let before = before.to_string();
            let after = after.to_string();
            document.set_trailing(after);
            before
        }
        None => String::new(),
    };
    key.leaf_decor_mut().set_prefix(prefix);
    let email = document["email"]
        .as_table_mut()
        .expect("[email] is a section");
    email.insert_formatted(&key, item.clone());
    // Each key keeps its place, and `to` goes just before the key at `place`,
    // or last: odd ranks for the keys that were there, an even one for `to`.
    let rank = |name: &str| match keys.iter().position(|key| key == name) {
        Some(at) => 2 * at + 1,
        None => 2 * place,
    };
    email.sort_values_by(|a, _, b, _| rank(a.get()).cmp(&rank(b.get())));
}

/// A comment after the last email key belongs to the following section's
/// prefix, or the document's trailing text. It still describes email.to.
enum EmailExample {
    BeforeKey(String),
    BeforeSection(isize),
    Trailing,
}

fn email_example(document: &DocumentMut) -> Option<EmailExample> {
    let email = document.get("email")?.as_table()?;
    if email.is_dotted() || email.is_implicit() {
        return None;
    }
    for (name, _) in email.iter() {
        let prefix = decor_prefix(email.key(name)?.leaf_decor());
        if prefix.lines().any(is_commented_out_email_to) {
            return Some(EmailExample::BeforeKey(name.to_string()));
        }
    }
    let position = email
        .position()
        .expect("an explicit section has a position");
    if let Some(next) = following_section(document.as_table(), position) {
        decor_prefix(next.decor())
            .lines()
            .any(is_commented_out_email_to)
            .then(|| EmailExample::BeforeSection(next.position().unwrap()))
    } else {
        document
            .trailing()
            .as_str()
            .unwrap_or("")
            .lines()
            .any(is_commented_out_email_to)
            .then_some(EmailExample::Trailing)
    }
}

/// Find the next written section, including nested Harness sections whose
/// headers can precede their parent's explicit header.
fn following_section(table: &toml_edit::Table, position: isize) -> Option<&toml_edit::Table> {
    let current = table
        .position()
        .filter(|at| *at > position && !table.is_implicit() && !table.is_dotted())
        .map(|_| table);
    table
        .iter()
        .filter_map(|(_, item)| item.as_table())
        .filter_map(|section| following_section(section, position))
        .chain(current)
        .min_by_key(|section| section.position().unwrap())
}

fn section_at(table: &mut toml_edit::Table, position: isize) -> Option<&mut toml_edit::Table> {
    if table.position() == Some(position) && !table.is_implicit() && !table.is_dotted() {
        return Some(table);
    }
    table
        .iter_mut()
        .filter_map(|(_, item)| item.as_table_mut())
        .find_map(|section| section_at(section, position))
}

fn take_email_example(decor: &mut toml_edit::Decor) -> String {
    let prefix = decor_prefix(decor);
    let (before, after) = split_email_example(&prefix);
    decor.set_prefix(after);
    before.to_string()
}

/// Remove only the example's line, retaining the adjacent comments on
/// either side for the inserted recipient and the next key or section.
fn split_email_example(prefix: &str) -> (&str, &str) {
    let mut end = 0;
    for line in prefix.split_inclusive('\n') {
        let start = end;
        end += line.len();
        if is_commented_out_email_to(line) {
            return (&prefix[..start], &prefix[end..]);
        }
    }
    unreachable!("the decoration contains the email example")
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

/// With no `email.to` in `email`, and no commented-out one either, add the
/// commented-out example line from
/// `defaults`, the `[email]` section of `DEFAULTS`, just before `email.from`:
/// `email.to` has no default to write.
fn note_email_to(email: &mut toml_edit::Table, defaults: &toml_edit::Table, has_example: bool) {
    if email.contains_key("to") || has_example {
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
quiet_skips = false   # a skipped Pass prints nothing, leaving only its Activity log line; default false

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3

[pickup]
limit = 3   # how many open issues labelled in-progress stop a Pickup run taking another; default 3

[harness]
default = "claude"   # the Harness every Run's sessions run on, claude, codex, agy, grok, muse or opencode; default claude

[harness.claude]
model = ""    # the Model Claude Code's sessions run on; default blank, for Claude Code's own
effort = ""   # how hard that Model reasons; default blank, for Claude Code's own

[harness.codex]
model = ""    # the Model Codex's sessions run on; default blank, for Codex's own
effort = ""   # how hard that Model reasons; default blank, for Codex's own

[harness.agy]
model = ""    # the Model Antigravity CLI's sessions run on; default blank, for agy's own
effort = ""   # how hard that Model reasons; default blank, for agy's own

[harness.grok]
model = ""    # the Model Grok Build's sessions run on; default blank, for Grok Build's own
effort = ""   # how hard that Model reasons; default blank, for Grok Build's own

[harness.muse]
model = ""    # the Model Muse Code's sessions run on; default blank, for Muse Code's own
effort = ""   # how hard that Model reasons; default blank, for Muse Code's own

[harness.opencode]
model = ""    # the Model OpenCode's sessions run on, provider/model; default blank, for OpenCode's own
effort = ""   # the Model's variant, passed as #effort; default blank, for OpenCode's own
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
                "harness.codex.effort",
                "harness.agy.model",
                "harness.agy.effort",
                "harness.grok.model",
                "harness.grok.effort",
                "harness.muse.model",
                "harness.muse.effort",
                "harness.opencode.model",
                "harness.opencode.effort"
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
        let path = Path::new("/home/me/.thirdshift/config.toml");
        let home = Path::new("/home/me");
        assert_eq!(
            UserConfigDocument::defaults(path, home, None)
                .unwrap()
                .render(None),
            DEFAULTS
        );
        let document =
            UserConfigDocument::defaults(path, home, Some("o\"brien@example.com".to_string()))
                .unwrap();
        assert_eq!(
            document.settings().email.to.as_deref(),
            Some("o\"brien@example.com")
        );
        let text = document.render(None);
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
                ..harness::Settings::default()
            }
        );
        assert_eq!(parse("").unwrap().harness, harness::Settings::default());
    }

    #[test]
    fn a_harness_setting_thirdshift_cant_use_is_an_error_naming_it() {
        for (text, error) in [
            (
                "[harness]\ndefault = \"gemini\"\n",
                "harness.default must be claude or codex or agy or grok or muse or opencode, not \"gemini\"",
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
            let prepared_error = UserConfigDocument::existing(
                text,
                Path::new("/home/me/.thirdshift/config.toml"),
                Path::new("/home/me"),
            )
            .err()
            .unwrap();
            assert_eq!(format!("{prepared_error:#}"), error_text);
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
            let prepared_error = UserConfigDocument::existing(
                text,
                Path::new("/home/me/.thirdshift/config.toml"),
                Path::new("/home/me"),
            )
            .err()
            .unwrap();
            assert_eq!(format!("{prepared_error:#}"), error);
            assert!(error.contains(named), "{text:?}: {error}");
            assert!(
                error.contains("/home/me/.thirdshift/config.toml"),
                "{text:?}: {error}"
            );
        }
    }

    #[test]
    fn preparing_invalid_section_shapes_returns_the_runs_path_aware_error() {
        let path = Path::new("/home/me/.thirdshift/config.toml");
        let home = Path::new("/home/me");
        for text in [
            "merge = true\n",
            "harness = false\n",
            "[harness]\nclaude = true\n",
        ] {
            let error = UserConfigDocument::existing(text, path, home)
                .err()
                .unwrap();
            assert_eq!(
                format!("{error:#}"),
                format!("{:#}", parse(text).unwrap_err())
            );
        }
    }

    #[test]
    fn an_empty_document_has_completed_defaults_and_accepts_semantic_changes_directly() {
        let path = Path::new("/users/jane/.thirdshift/config.toml");
        let home = Path::new("/users/jane");
        let document = UserConfigDocument::existing("", path, home).unwrap();
        let current = document.settings();
        assert_eq!(current.harness.default, Some(Harness::Claude));
        assert_eq!(
            current.email.from.as_deref(),
            Some(crate::email::DEFAULT_FROM)
        );
        assert_eq!(current.email.to, None);
        assert_eq!(current.harness.claude, ModelAndEffort::default());
        assert_eq!(current.logs_dir, home.join(".thirdshift/logs"));

        let text = document.render(Some(UserConfigChanges {
            merge_always: true,
            base_fix: true,
            launch_pull: true,
            notifications: Some(NotificationAddresses {
                to: "me@example.com".to_string(),
                from: "ts@example.com".to_string(),
            }),
            harness: Some((
                Harness::Codex,
                ModelAndEffort {
                    model: Some("gpt-6.1-sol".to_string()),
                    effort: Some("max".to_string()),
                },
            )),
        }));
        let config = UserConfig::parse(&text, path, home).unwrap();
        assert!(config.merge_always && config.base_fix && config.launch_pull);
        assert!(config.email.always);
        assert_eq!(config.email.to.as_deref(), Some("me@example.com"));
        assert_eq!(config.email.from.as_deref(), Some("ts@example.com"));
        assert_eq!(config.harness.default, Some(Harness::Codex));
        assert_eq!(config.harness.codex.model.as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(config.harness.codex.effort.as_deref(), Some("max"));
        assert_eq!(config.logs_dir, home.join(".thirdshift/logs"));
        assert_eq!(config.spec_parallel.get(), 3);
        assert_eq!(config.pickup_limit.get(), 3);
    }

    /// `text` completed, after checking a Run reads it with the settings it
    /// had, plus the defaults for the keys it lacked, which change nothing a
    /// Run does.
    fn completed(text: &str) -> String {
        let completed = document(text).render(None);
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
        let again = document(&completed).render(None);
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
            for name in Harness::ALL.map(Harness::name) {
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
    fn document(text: &str) -> UserConfigDocument {
        UserConfigDocument::existing(
            text,
            Path::new("/home/me/.thirdshift/config.toml"),
            Path::new("/home/me"),
        )
        .unwrap()
    }

    // Semantic edits through the prepared document.

    fn notifications_to(to: &str) -> UserConfigChanges {
        UserConfigChanges {
            harness: None,
            merge_always: false,
            base_fix: false,
            launch_pull: false,
            notifications: Some(NotificationAddresses {
                to: to.to_string(),
                from: crate::email::DEFAULT_FROM.to_string(),
            }),
        }
    }

    #[test]
    fn an_answered_address_takes_the_place_of_the_commented_out_line() {
        let answered = document(DEFAULTS).render(Some(notifications_to("me@example.com")));
        let expected = DEFAULTS.replace(NO_EMAIL_TO, "to = \"me@example.com\"           # where email goes when the command names no address; no default").replace(
            "always = false                  #",
            "always = true                   #",
        );
        assert_eq!(answered, expected);
    }

    #[test]
    fn an_answered_address_keeps_the_lines_around_the_commented_out_one() {
        let text =
            "[email]\nalways = false\n\n# mine\n# to = \"x@y.z\"\n# more\nfrom = \"a@b.c\"\n";
        let answered = document(text).render(Some(notifications_to("me@example.com")));
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
    #[test]
    fn completed_settings_expand_log_paths_and_leave_blank_model_and_effort_unset() {
        let home = Path::new("/users/jane");
        let path = home.join(".thirdshift/config.toml");
        for (dir, expected) in [
            ("~/elsewhere/logs", "/users/jane/elsewhere/logs"),
            ("~", "/users/jane"),
            ("/var/log/thirdshift", "/var/log/thirdshift"),
        ] {
            let text = format!("[logs]\ndir = {dir:?}\n[harness.codex]\nmodel = ''\neffort = ''\n");
            let document = UserConfigDocument::existing(&text, &path, home).unwrap();
            let settings = document.settings();
            assert_eq!(settings.logs_dir, Path::new(expected));
            assert_eq!(settings.harness.default, Some(Harness::Claude));
            assert_eq!(
                settings.email.from.as_deref(),
                Some(crate::email::DEFAULT_FROM)
            );
            for harness in Harness::ALL {
                assert_eq!(settings.harness.of(harness), &ModelAndEffort::default());
            }
        }
    }

    #[test]
    fn equal_answers_keep_literal_spelling_and_decoration() {
        let text = DEFAULTS
            .replace("default = \"claude\"", "default = \"\\u0063laude\"")
            .replace("model = \"\"", "model = ''")
            .replace("effort = \"\"", "effort = ''")
            .replace("parallel = 3", "parallel = 1_000")
            .replace(
                "from = \"onboarding@resend.dev\"",
                "from = 'onboarding@resend.dev'",
            )
            .replace(
                "always = false                  #",
                "always = true                   #",
            )
            .replace(NO_EMAIL_TO, "to = 'me@example.com'  # my inbox");
        let mut changes = notifications_to("me@example.com");
        changes.harness = Some((Harness::Claude, ModelAndEffort::default()));

        assert_eq!(document(&text).render(Some(changes)), text);
    }

    #[test]
    fn changed_values_keep_surrounding_comments_and_adjust_comment_columns() {
        let text = "# top\n[merge]\n# before\nalways = false   # merge\n# after\n\n\
                    [harness.codex]\nmodel = 'old'       # model\neffort = 'high'      # effort\n";
        let changes = UserConfigChanges {
            merge_always: true,
            base_fix: false,
            launch_pull: false,
            notifications: None,
            harness: Some((
                Harness::Codex,
                ModelAndEffort {
                    model: Some("much-longer-model".to_string()),
                    effort: None,
                },
            )),
        };
        let rendered = document(text).render(Some(changes));
        assert!(
            rendered.starts_with("# top\n[merge]\n# before\nalways = true    # merge\n# after\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("model = \"much-longer-model\" # model\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("effort = \"\"          # effort\n"),
            "{rendered}"
        );
    }
    #[test]
    fn direct_edits_of_partial_dotted_inline_and_implicit_sections_preserve_other_harnesses() {
        for text in [
            "# dotted\nmerge.always = false\nbase.fix = false\nlaunch.pull = false\n\
             email.always = false\nemail.to = 'saved@example.com'\n\
             harness.default = 'claude'\nharness.codex.model = 'old'\n\
             harness.codex.effort = 'high'\nharness.claude.model = 'opus'\n",
            "# inline\nmerge = { always = false }\nbase = { fix = false }\n\
             launch = { pull = false }\nemail = { always = false, to = 'saved@example.com' }\n\
             harness = { default = 'claude', codex = { model = 'old', effort = 'high' }, claude = { model = 'opus' } }\n",
            "[harness]\ndefault = 'claude'\ncodex = { model = 'old', effort = 'high' }\n\
             claude = { model = 'opus' }\n",
            "[harness.codex]\nmodel = 'old' # mine\neffort = 'high'\n\
             [harness.claude]\nmodel = 'opus'\n",
        ] {
            let before = parse(text).unwrap();
            let changes = UserConfigChanges {
                merge_always: true,
                base_fix: true,
                launch_pull: true,
                notifications: Some(NotificationAddresses {
                    to: "o\"brien@example.com".to_string(),
                    from: "ts@example.com".to_string(),
                }),
                harness: Some((Harness::Codex, ModelAndEffort::default())),
            };
            let rendered = document(text).render(Some(changes));
            let config = parse(&rendered).unwrap();
            assert!(
                config.merge_always && config.base_fix && config.launch_pull,
                "{rendered}"
            );
            assert!(config.email.always, "{rendered}");
            assert_eq!(config.email.to.as_deref(), Some("o\"brien@example.com"));
            assert_eq!(config.email.from.as_deref(), Some("ts@example.com"));
            assert_eq!(config.harness.default, Some(Harness::Codex));
            let table: toml::Table = rendered.parse().unwrap();
            assert_eq!(table["harness"]["codex"]["model"].as_str(), Some(""));
            assert_eq!(table["harness"]["codex"]["effort"].as_str(), Some(""));
            for harness in Harness::ALL.into_iter().filter(|h| *h != Harness::Codex) {
                assert_eq!(
                    config.harness.of(harness),
                    before.harness.of(harness),
                    "{rendered}"
                );
            }
            assert_eq!(config.logs_dir, before.logs_dir);
            assert_eq!(config.quiet_skips, before.quiet_skips);
            assert_eq!(config.spec_parallel, before.spec_parallel);
            assert_eq!(config.pickup_limit, before.pickup_limit);
            if let Some(comment) = text.lines().next().filter(|line| line.starts_with('#')) {
                assert!(rendered.starts_with(comment), "{rendered}");
            }
            // The completed rendering still uses the user's representation.
            if text.contains("merge.always") {
                assert!(rendered.contains("merge.always = true\n"), "{rendered}");
                assert!(
                    rendered.contains("harness.codex.model = \"\"\n"),
                    "{rendered}"
                );
            } else if text.contains("harness = {") {
                assert!(
                    rendered.contains("merge = { always = true }\n"),
                    "{rendered}"
                );
                assert!(!rendered.contains("[harness]"), "{rendered}");
            } else if text.contains("codex = {") {
                assert!(
                    rendered.contains("codex = { model = \"\", effort = \"\" }\n"),
                    "{rendered}"
                );
            } else {
                assert!(
                    rendered
                        .starts_with("[harness.codex]\nmodel = \"\"    # mine\neffort = \"\"\n"),
                    "{rendered}"
                );
                assert!(
                    rendered.find("[harness.claude]").unwrap()
                        < rendered.find("[harness]\n").unwrap()
                );
            }
            assert_eq!(document(&rendered).render(None), rendered);
        }
    }
    #[test]
    fn disabling_notifications_keeps_saved_addresses_harnesses_and_unasked_settings() {
        let mut text = "[email]\nalways = true # enabled\nto = 'saved@example.com' # inbox\n\
                        from = 'saved@example.net' # sender\n\
                        [logs]\ndir = '~/custom'\n[activity]\nquiet_skips = true\n\
                        [spec]\nparallel = 1_000 # capacity\n[pickup]\nlimit = 7\n\
                        [harness]\ndefault = 'muse' # chosen\n"
            .to_string();
        for harness in Harness::ALL {
            text.push_str(&format!(
                "[harness.{}]\nmodel = 'saved-model' # model\neffort = 'saved-effort' # effort\n",
                harness.name()
            ));
        }
        let before = parse(&text).unwrap();
        let changes = UserConfigChanges {
            merge_always: true,
            base_fix: false,
            launch_pull: true,
            notifications: None,
            harness: None,
        };
        let rendered = document(&text).render(Some(changes));
        let config = parse(&rendered).unwrap();
        assert!(!config.email.always);
        assert_eq!(config.email.to, before.email.to);
        assert_eq!(config.email.from, before.email.from);
        assert_eq!(config.harness, before.harness);
        assert_eq!(config.logs_dir, before.logs_dir);
        assert_eq!(config.quiet_skips, before.quiet_skips);
        assert_eq!(config.spec_parallel, before.spec_parallel);
        assert_eq!(config.pickup_limit, before.pickup_limit);
        assert!(config.merge_always && config.launch_pull);
        assert!(!config.base_fix);
        assert!(rendered.starts_with("[email]\nalways = false # enabled\nto = 'saved@example.com' # inbox\nfrom = 'saved@example.net' # sender\n"), "{rendered}");
        for line in text
            .lines()
            .filter(|line| *line != "always = true # enabled")
        {
            assert!(
                rendered.lines().any(|kept| kept == line),
                "lost {line:?}:\n{rendered}"
            );
        }
    }

    #[test]
    fn enabling_notifications_quotes_the_recipient_and_replaces_the_example_in_place() {
        let text = "[email]\nalways = false\n# mine\n# to = 'someday@example.com'\n# more\nfrom = 'onboarding@resend.dev'\n";
        let to = "o\"brien\\team@example.com";
        let rendered = document(text).render(Some(notifications_to(to)));
        let config = parse(&rendered).unwrap();
        assert!(config.email.always);
        assert_eq!(config.email.to.as_deref(), Some(to));
        let lines: Vec<_> = rendered.lines().collect();
        assert_eq!(lines[..3], ["[email]", "always = true", "# mine"]);
        assert!(lines[3].starts_with("to = "), "{rendered}");
        assert!(
            rendered.contains("\n# more\nfrom = 'onboarding@resend.dev'\n"),
            "{rendered}"
        );
        assert!(!rendered.contains("# to ="), "{rendered}");
        assert_eq!(document(&rendered).render(None), rendered);
    }
    #[test]
    fn enabling_notifications_replaces_examples_at_section_and_document_boundaries() {
        let email_start = DEFAULTS.find("[email]\n").unwrap();
        let email_end = DEFAULTS.find("[logs]\n").unwrap();
        let email_last = format!(
            "{}{}\n[email]\nalways = false\nfrom = 'onboarding@resend.dev'\n# mine\n# to = 'saved@example.com'\n# more\n",
            &DEFAULTS[..email_start],
            &DEFAULTS[email_end..]
        );
        for text in [
            "[email]\nalways = false\nfrom = 'onboarding@resend.dev'\n# mine\n# to = 'saved@example.com'\n# more\n",
            "[email]\nalways = false\nfrom = 'onboarding@resend.dev'\n# mine\n# to = 'saved@example.com'\n# more\n[merge]\nalways = false\n",
            "[email]\nalways = false\nfrom = 'onboarding@resend.dev'\n# mine\n# to = 'saved@example.com'\n# more\n[harness.codex]\nmodel = 'saved'\n",
            "[email]\n# mine\n# to = 'saved@example.com'\n# more\n",
            "[email]\n# mine\n# to = 'saved@example.com'\n# more\n[merge]\nalways = false\n",
            &email_last,
        ] {
            let completed = document(text).render(None);
            assert_eq!(completed.matches("# to =").count(), 1, "{completed}");
            let rendered = document(text).render(Some(notifications_to("me@example.com")));
            let lines: Vec<_> = rendered.lines().collect();
            let to = lines
                .iter()
                .position(|line| line.starts_with("to = "))
                .unwrap();
            assert_eq!(lines[to - 1], "# mine", "{rendered}");
            assert_eq!(lines[to + 1], "# more", "{rendered}");
            assert!(!rendered.contains("# to ="), "{rendered}");
            let settings = parse(&rendered).unwrap();
            assert!(settings.email.always);
            assert_eq!(settings.email.to.as_deref(), Some("me@example.com"));
            assert_eq!(document(&rendered).render(None), rendered);
        }
    }
    #[test]
    fn quoted_email_headers_keep_one_example_and_render_idempotently() {
        for header in [r#"["email"]"#, "['email']", r#"["em\u0061il"]"#] {
            let text = format!(
                "{header}\nalways = false\n# mine\n# to = 'saved@example.com'\n# more\nfrom = 'onboarding@resend.dev'\n"
            );
            let completed = document(&text).render(None);
            assert_eq!(completed.matches("# to =").count(), 1, "{completed}");
            assert!(completed.starts_with(&text), "{completed}");
            assert_eq!(document(&completed).render(None), completed);

            let rendered = document(&text).render(Some(notifications_to("me@example.com")));
            assert!(rendered.starts_with(header), "{rendered}");
            assert!(!rendered.contains("# to ="), "{rendered}");
            let lines: Vec<_> = rendered.lines().collect();
            let to = lines
                .iter()
                .position(|line| line.starts_with("to = "))
                .unwrap();
            assert_eq!(lines[to - 1], "# mine", "{rendered}");
            assert_eq!(lines[to + 1], "# more", "{rendered}");
            let settings = parse(&rendered).unwrap();
            assert!(settings.email.always);
            assert_eq!(settings.email.to.as_deref(), Some("me@example.com"));
            assert_eq!(document(&rendered).render(None), rendered);
        }
    }
}
