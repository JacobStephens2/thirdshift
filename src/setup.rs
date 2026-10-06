//! Setup: writing the User config by answering a few questions, from
//! `thirdshift setup` or the first Run's offer of it, and the Credentials
//! when a Resend API key is entered.
//!
//! Its rules are here: whether to ask, the questions and their defaults, the
//! suggested address, what is written and in what order, and which failures
//! the offer carries on through. Parsing, the defaults and completing a User
//! config are the User config module's. Everything Setup does outside
//! itself goes through [`Outside`]: [`OnMachine`] does each on the terminal,
//! the Harnesses' CLIs, `gh`, `git`, Resend and the files under the home
//! folder; `Scripted`, in tests, from a script, recording each call.

pub(crate) mod questions;

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};
use signal_hook::SigId;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::low_level;
use toml_edit::{DocumentMut, Item};

use crate::config::{self, EmailSettings, UserConfig};
use crate::git::Git;
use crate::harness::{Catalog, Choice, ChosenBy, Harness, ModelAndEffort};
use crate::resend_key::{Credentials, Source};
use crate::{email, github, progress};

use questions::Answers;

/// Setup: write the User config. From a terminal, the Setup questions come
/// first, each with the current value as its default answer, and the answers
/// are written, with `base.fix`, asked about only when every Run is a Merge
/// run, otherwise at its default; with no terminal, nothing is asked, and
/// every setting is at its default, with `email.to` as the suggested
/// address, if there is one.
/// An existing User config is edited in place, once it parses as a Run would
/// parse it: its comments and key order stay, as do the values Setup didn't
/// ask about, and each key it lacks is added at its default, so Setup with no
/// terminal never resets a configured machine. A Resend API key the user
/// gave is saved in the Credentials once the User config is written; with no
/// terminal, the Credentials are never read or written. Nothing is written
/// until the last answer is in. A test email, if the user asked for one, goes
/// once the files are written, as `email-test` sends it. Returns the line
/// saying what was done last.
pub fn setup() -> Result<String> {
    let (home, path) = config::home_and_path()?;
    run_setup(&mut OnMachine::new(&home), &home, &path)
}

/// The first Run's offer of Setup, made only from a terminal and only when
/// there is no User config. Yes asks the Setup questions and writes the
/// answers, and the Resend API key to the Credentials if the user gave one;
/// no writes every setting at its default, as `setup` with no terminal does,
/// and leaves the Credentials alone. Either way the Run then loads what was
/// written. A User config or Credentials that can't be written, or a test
/// email that can't go, is a warning, so the Run carries on; Credentials a
/// Run would refuse, or stdin closing before the last answer, end the command
/// before any work, with nothing written.
pub fn offer() -> Result<()> {
    let (home, path) = config::home_and_path()?;
    run_offer(&mut OnMachine::new(&home), &home, &path)
}

/// [`setup`], for the User config at `path` under `home`.
fn run_setup(outside: &mut impl Outside, home: &Path, path: &Path) -> Result<String> {
    let existing = outside
        .read_user_config(path)
        .with_context(|| format!("can't read {}", path.display()))?;
    let text = match &existing {
        Some(text) => {
            UserConfig::parse(text, path, home)?;
            config::complete(text)?
        }
        None => config::with_email_to(suggested_address(outside)),
    };
    let asking = outside.has_terminal();
    let (text, answered) = ask_if(asking, outside, text, path, home)?;
    let asked = answered.is_some();
    let mut written = match &existing {
        None => {
            write_new(outside, path, &text)?;
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
            replace(outside, path, &text)?;
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
    let Some(answers) = &answered else {
        return Ok(written);
    };
    if let Some(key) = answers.key() {
        outside.step(written);
        written = outside.save_key(key)?;
    }
    if !answers.send_test() {
        return Ok(written);
    }
    outside.step(written);
    let config = UserConfig::parse(&text, path, home)?;
    outside.send_test(&config.email)
}

/// [`offer`], for the User config at `path` under `home`.
fn run_offer(outside: &mut impl Outside, home: &Path, path: &Path) -> Result<()> {
    if !outside.has_terminal() || outside.user_config_exists(path) {
        return Ok(());
    }
    let accepted = questions::offer(outside, path)?;
    let text = config::with_email_to(suggested_address(outside));
    let (text, answered) = ask_if(accepted, outside, text, path, home)?;
    if let Err(error) = write_new(outside, path, &text) {
        outside.step(format!("warning: {error:#}; carrying on with the defaults"));
        return Ok(());
    }
    let Some(answers) = &answered else {
        outside.step(format!(
            "wrote the User config {} with every setting at its default; \
             thirdshift setup changes it",
            path.display()
        ));
        return Ok(());
    };
    outside.step(format!("wrote the User config {}", path.display()));
    if let Some(key) = answers.key() {
        match outside.save_key(key) {
            Ok(wrote) => outside.step(wrote),
            Err(error) => outside.step(format!("warning: {error:#}")),
        }
    }
    if answers.send_test() {
        let sent = UserConfig::parse(&text, path, home)
            .and_then(|config| outside.send_test(&config.email));
        match sent {
            Ok(sent) => outside.step(sent),
            Err(error) => outside.step(format!("warning: {error:#}")),
        }
    }
    Ok(())
}

/// `text` with the answers to the Setup questions, and the answers, if
/// `asking`, as [`ask`] asks them; otherwise `text` as it is.
fn ask_if(
    asking: bool,
    outside: &mut impl Outside,
    text: String,
    path: &Path,
    home: &Path,
) -> Result<(String, Option<Answers>)> {
    if !asking {
        return Ok((text, None));
    }
    let (text, answers) = ask(outside, &text, path, home)?;
    Ok((text, Some(answers)))
}

/// Ask the Setup questions, with the settings in `text`, the User config at
/// `path` under `home`, and the key in the Credentials as the default
/// answers. Credentials a Run would refuse are refused before any question.
/// Returns `text` with the answers, and the answers.
fn ask(
    outside: &mut impl Outside,
    text: &str,
    path: &Path,
    home: &Path,
) -> Result<(String, Answers)> {
    let current = UserConfig::parse(text, path, home)?;
    let found = outside.find_key()?;
    let answers = questions::ask(outside, &current, found)?;
    let text = with_answers(text, &answers)?;
    Ok((text, answers))
}

/// Write `text` as the User config at `path`, where there is none yet.
fn write_new(outside: &mut impl Outside, path: &Path, text: &str) -> Result<()> {
    outside
        .write_new_user_config(path, text)
        .with_context(|| cant_write(path))
}

/// Replace the User config at `path` with `text`.
fn replace(outside: &mut impl Outside, path: &Path, text: &str) -> Result<()> {
    outside
        .replace_user_config(path, text)
        .with_context(|| cant_write(path))
}

/// What a write of the User config at `path` that failed says.
fn cant_write(path: &Path) -> String {
    format!("can't write {}", path.display())
}

/// The address Setup suggests for `email.to`: the public email of the user's
/// GitHub profile, else the global git `user.email`, unless that is a
/// `@users.noreply.github.com` address, which can't receive mail. Only Setup
/// looks it up; a Run never falls back to it.
fn suggested_address(outside: &mut impl Outside) -> Option<String> {
    let github = outside.github_email().ok().flatten();
    github.filter(|email| !email.is_empty()).or_else(|| {
        outside.git_email().ok().filter(|email| {
            !email.is_empty()
                && !email
                    .to_ascii_lowercase()
                    .ends_with("@users.noreply.github.com")
        })
    })
}

/// `text`, a User config with every key, or a commented-out `email.to`, with
/// the values `answers` gives. Only those values change: the spacing and
/// comments around each stay, as does everything else in `text`.
fn with_answers(text: &str, answers: &Answers) -> Result<String> {
    let mut document: DocumentMut = text.parse().context("can't parse the User config")?;
    set(&mut document, "merge", "always", answers.merge_always);
    set(&mut document, "base", "fix", answers.base_fix);
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
    if let Some((harness, chosen)) = &answers.harness {
        set(&mut document, "harness", "default", harness.name());
        let section = format!("harness.{}", harness.name());
        for (key, value) in [("model", &chosen.model), ("effort", &chosen.effort)] {
            set(&mut document, &section, key, value.as_deref().unwrap_or(""));
        }
    }
    Ok(document.to_string())
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
    let suffix = config::decor_suffix(&decor);
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
    let Some(email) = email.as_table_mut().filter(|email| !email.is_dotted()) else {
        let settings = email.as_table_like_mut().expect("[email] is a section");
        settings.insert("to", Item::Value(to.into()));
        return;
    };
    let example: DocumentMut = config::with_email_to(Some(to.to_string()))
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
        let prefix = config::decor_prefix(next.leaf_decor());
        let mut end = 0;
        let found = prefix.split_inclusive('\n').find_map(|line| {
            let start = end;
            end += line.len();
            config::is_commented_out_email_to(line).then_some((start, end))
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

/// What Setup does outside itself: the terminal it asks on, the Harnesses'
/// CLIs, the Credentials, the lookups behind the suggested address, the User
/// config file, the test email, and where its progress lines go.
pub(crate) trait Outside {
    /// Whether there is someone to ask: stdin and stderr are both terminals.
    fn has_terminal(&mut self) -> bool;
    /// Show `prompt` and read one answer, trimmed, or `None` if stdin closed.
    fn read(&mut self, prompt: &str) -> Result<Option<String>>;
    /// [`Outside::read`], with what is typed never shown.
    fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>>;
    /// Show `line` on the terminal.
    fn say(&mut self, line: String);
    /// Whether sessions can run on `harness` here.
    fn installed(&mut self, harness: Harness) -> bool;
    /// Make Claude's test call on the Model and Effort `chosen`, as a Run
    /// checks them.
    fn test_call(&mut self, chosen: &ModelAndEffort) -> Result<()>;
    /// Read Codex's catalog of Models.
    fn codex_catalog(&mut self) -> Result<Catalog>;
    /// Read Antigravity CLI's free Model catalog.
    fn agy_catalog(&mut self) -> Result<crate::harness::agy::Catalog>;
    /// Read Grok Build's catalog, refreshed by grok models.
    fn grok_catalog(&mut self) -> Result<crate::harness::grok::Catalog>;
    /// Check Muse's settings against its cache, or with its minimal test call.
    fn muse_check(&mut self, chosen: &ModelAndEffort) -> Result<ModelAndEffort>;
    /// Check OpenCode with its minimal standalone test call.
    fn opencode_check(&mut self, chosen: &ModelAndEffort) -> Result<()>;
    /// Read the Credentials, as strictly as a Run does: where a Resend API
    /// key is found, if anywhere.
    fn find_key(&mut self) -> Result<Option<Source>>;
    /// Save `key` in the Credentials: the line saying where.
    fn save_key(&mut self, key: &str) -> Result<String>;
    /// The public email of the user's GitHub profile, if it has one.
    fn github_email(&mut self) -> Result<Option<String>>;
    /// The global git `user.email`.
    fn git_email(&mut self) -> Result<String>;
    /// The User config at `path`, or `None` if there is none.
    fn read_user_config(&mut self, path: &Path) -> Result<Option<String>>;
    /// Whether anything is at `path`, where the User config goes.
    fn user_config_exists(&mut self, path: &Path) -> bool;
    /// Write `text` as the User config at `path`, where there is none yet,
    /// creating its folder if it is missing.
    fn write_new_user_config(&mut self, path: &Path, text: &str) -> Result<()>;
    /// Replace the User config at `path` with `text`, all at once.
    fn replace_user_config(&mut self, path: &Path, text: &str) -> Result<()>;
    /// Send the test email, with `email`, as `email-test` sends it: the line
    /// saying it went.
    fn send_test(&mut self, email: &EmailSettings) -> Result<String>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// This machine, with its home folder `home`: the terminal on stdin and
/// stderr, the Harnesses' CLIs, `gh`, `git`, Resend, and the files under
/// `home`.
struct OnMachine<'a> {
    home: &'a Path,
    /// The Credentials as read, kept so a key is saved into what was read.
    credentials: Option<Credentials>,
}

impl<'a> OnMachine<'a> {
    fn new(home: &'a Path) -> Self {
        OnMachine {
            home,
            credentials: None,
        }
    }
}

impl Outside for OnMachine<'_> {
    fn has_terminal(&mut self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    fn read(&mut self, prompt: &str) -> Result<Option<String>> {
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "{prompt}");
        let _ = stderr.flush();
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            let _ = writeln!(stderr);
            return Ok(None);
        }
        Ok(Some(line.trim().to_string()))
    }

    /// With the terminal's echo off, so what is typed never shows.
    fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>> {
        let echo_off = EchoOff::new()?;
        let line = self.read(prompt)?;
        drop(echo_off);
        if line.is_some() {
            let _ = writeln!(std::io::stderr());
        }
        Ok(line)
    }

    fn say(&mut self, line: String) {
        let _ = writeln!(std::io::stderr(), "{line}");
    }

    fn installed(&mut self, harness: Harness) -> bool {
        harness.installed()
    }

    fn test_call(&mut self, chosen: &ModelAndEffort) -> Result<()> {
        crate::harness::claude::test_call(&Choice {
            harness: Harness::Claude,
            model: chosen.model.clone(),
            effort: chosen.effort.clone(),
            chosen_by: ChosenBy::UserConfig,
        })
    }

    fn codex_catalog(&mut self) -> Result<Catalog> {
        Catalog::read()
    }

    fn agy_catalog(&mut self) -> Result<crate::harness::agy::Catalog> {
        crate::harness::agy::Catalog::read()
    }

    fn grok_catalog(&mut self) -> Result<crate::harness::grok::Catalog> {
        crate::harness::grok::Catalog::read()
    }

    fn muse_check(&mut self, chosen: &ModelAndEffort) -> Result<ModelAndEffort> {
        crate::harness::muse::check_model_and_effort(chosen)
    }

    fn opencode_check(&mut self, chosen: &ModelAndEffort) -> Result<()> {
        crate::harness::opencode::check_model_and_effort(chosen)
    }

    fn find_key(&mut self) -> Result<Option<Source>> {
        let credentials = Credentials::read()?;
        let found = credentials.lookup().map(|key| key.source);
        self.credentials = Some(credentials);
        Ok(found)
    }

    fn save_key(&mut self, key: &str) -> Result<String> {
        let credentials = match self.credentials.take() {
            Some(credentials) => credentials,
            None => Credentials::read()?,
        };
        credentials.save(key)?;
        Ok(format!("wrote {credentials}"))
    }

    fn github_email(&mut self) -> Result<Option<String>> {
        github::profile_email()
    }

    fn git_email(&mut self) -> Result<String> {
        Git::new(self.home).run(&["config", "--global", "user.email"])
    }

    fn read_user_config(&mut self, path: &Path) -> Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn user_config_exists(&mut self, path: &Path) -> bool {
        !matches!(
            std::fs::symlink_metadata(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    }

    fn write_new_user_config(&mut self, path: &Path, text: &str) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
        Ok(())
    }

    fn replace_user_config(&mut self, path: &Path, text: &str) -> Result<()> {
        Ok(config::replace(path, text)?)
    }

    fn send_test(&mut self, email: &EmailSettings) -> Result<String> {
        Ok(email::send_test(None, email)?.to_string())
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

/// The terminal on stdin with its echo off, until this is dropped. Ctrl-C,
/// SIGTERM or SIGHUP meanwhile turn echo back on, then end the command as
/// they would anywhere else in Setup, so it never leaves the shell blind.
struct EchoOff {
    saved: libc::termios,
    handlers: Vec<SigId>,
}

impl EchoOff {
    fn new() -> Result<Self> {
        // SAFETY: an all-zero termios is a valid value for tcgetattr to fill.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fd 0 is open, and `saved` is a valid termios to fill.
        if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("can't read the terminal's settings");
        }
        let mut echo_off = EchoOff {
            saved,
            handlers: Vec::new(),
        };
        for signal in [SIGINT, SIGTERM, SIGHUP] {
            // SAFETY: the handler only calls tcsetattr and signal-hook's
            // emulate_default_handler, both async-signal-safe.
            let handler = unsafe {
                low_level::register(signal, move || {
                    libc::tcsetattr(0, libc::TCSANOW, &saved);
                    let _ = low_level::emulate_default_handler(signal);
                })
            }
            .context("could not install the signal handler")?;
            echo_off.handlers.push(handler);
        }
        let mut hidden = saved;
        hidden.c_lflag &= !libc::ECHO;
        // SAFETY: fd 0 is open, and `hidden` is a valid termios.
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &hidden) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("can't turn the terminal's echo off");
        }
        Ok(echo_off)
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        // SAFETY: fd 0 is open, and `saved` is the termios read from it.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.saved) };
        for handler in self.handlers.drain(..) {
            low_level::unregister(handler);
        }
    }
}

#[cfg(test)]
mod scripted {
    use std::collections::VecDeque;

    use anyhow::{anyhow, bail};

    use super::*;

    /// What `codex debug models` prints: a few Models, each with its display
    /// name and the Efforts it supports.
    pub const CATALOG: &str = r#"{"models": [
      {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1-Sol", "visibility": "list",
       "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}, {"effort": "ultra"}]},
      {"slug": "gpt-6-luna", "display_name": "GPT-6-Luna", "visibility": "list",
       "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}]},
      {"slug": "gpt-5.5", "display_name": "GPT-5.5", "visibility": "list",
       "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}]}
    ]}"#;

    /// Where the Credentials are.
    pub const CREDENTIALS: &str = "/home/me/.thirdshift/credentials.toml";

    /// What the test email's send says once Resend accepts it.
    pub const SENT: &str = "accepted by Resend; check your inbox";

    /// A call Setup made, in the order it made it.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Call {
        /// It showed this prompt and read an answer.
        Ask(String),
        /// It showed this prompt and read a hidden answer.
        AskHidden(String),
        /// It showed this line on the terminal.
        Say(String),
        /// It made Claude's test call on this Model and Effort.
        Tested(ModelAndEffort),
        MuseChecked(ModelAndEffort),
        /// It wrote this new User config.
        WriteNew(String),
        /// It replaced the User config with this.
        Replace(String),
        /// It saved this key in the Credentials.
        SaveKey(String),
        /// It sent the test email to `to`, from `from`.
        SendTest {
            to: Option<String>,
            from: Option<String>,
        },
        /// It handed on this progress line.
        Step(String),
    }

    /// A write, save or send that can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fails {
        WriteNew,
        Replace,
        SaveKey,
        SendTest,
    }

    /// The outside world from a script: the answers to give, in order, to
    /// plain and hidden prompts alike, each with part of the prompt it
    /// answers; which Harnesses are installed, and Claude's test calls'
    /// results in turn; Codex's catalog; where a key is found; the two
    /// emails; the User config, if any; and what fails.
    pub struct Scripted {
        pub terminal: bool,
        /// Each answer, with part of the prompt it is for. Once they run
        /// out, stdin is closed.
        pub answers: VecDeque<(&'static str, &'static str)>,
        pub installed: Vec<Harness>,
        /// Claude's refusal of each test call in turn, if it refuses it;
        /// once they run out, it takes each.
        pub test_calls: VecDeque<Option<&'static str>>,
        /// The catalog's JSON, or what reading it fails with.
        pub catalog: Result<&'static str, &'static str>,
        pub agy_catalog: Result<&'static str, &'static str>,
        /// Grok's text model list and refreshed effort cache, or a read failure.
        pub grok_catalog: Result<(&'static str, &'static str), &'static str>,
        pub muse_checks: VecDeque<Result<ModelAndEffort, &'static str>>,
        /// Where a key is found, or what reading the Credentials fails with.
        pub key: Result<Option<Source>, &'static str>,
        pub github_email: Result<Option<&'static str>, &'static str>,
        pub git_email: Result<&'static str, &'static str>,
        /// The User config's text, if there is one, which writes change.
        pub user_config: Option<String>,
        pub failing: Vec<Fails>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
    }

    impl Scripted {
        /// A terminal answering `answers` in order, then closing stdin,
        /// with every Harness installed, no key, no email to suggest and no
        /// User config.
        pub fn answering(answers: &[(&'static str, &'static str)]) -> Self {
            Scripted {
                terminal: true,
                answers: answers.iter().copied().collect(),
                installed: Harness::ALL.to_vec(),
                test_calls: VecDeque::new(),
                catalog: Ok(CATALOG),
                agy_catalog: Ok(
                    "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\n",
                ),
                grok_catalog: Ok((
                    include_str!("../tests/fixtures/grok-models.txt"),
                    include_str!("../tests/fixtures/grok-models.json"),
                )),
                muse_checks: VecDeque::new(),
                key: Ok(None),
                github_email: Ok(None),
                git_email: Err("git config --global user.email: exit status 1"),
                user_config: None,
                failing: Vec::new(),
                calls: Vec::new(),
            }
        }

        /// No terminal, with the rest as [`Scripted::answering`] has it.
        pub fn unattended() -> Self {
            Scripted {
                terminal: false,
                ..Scripted::answering(&[])
            }
        }

        /// Every prompt shown, plain or hidden, in order.
        pub fn prompts(&self) -> Vec<&str> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Ask(prompt) | Call::AskHidden(prompt) => Some(prompt.as_str()),
                    _ => None,
                })
                .collect()
        }

        /// Every line said on the terminal, in order.
        pub fn said(&self) -> Vec<&str> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Say(line) => Some(line.as_str()),
                    _ => None,
                })
                .collect()
        }

        /// Every write, save, send and progress line, in order.
        pub fn effects(&self) -> Vec<&Call> {
            self.calls
                .iter()
                .filter(|call| {
                    !matches!(
                        call,
                        Call::Ask(_) | Call::AskHidden(_) | Call::Say(_) | Call::Tested(_)
                    )
                })
                .collect()
        }

        /// Every test call Claude was given, in order.
        pub fn test_calls(&self) -> Vec<&ModelAndEffort> {
            self.calls
                .iter()
                .filter_map(|call| match call {
                    Call::Tested(chosen) => Some(chosen),
                    _ => None,
                })
                .collect()
        }

        /// The next answer, checking `prompt` is the one it is for, or
        /// `None` if stdin is closed.
        fn answer(&mut self, prompt: &str) -> Option<String> {
            let (expected, answer) = self.answers.pop_front()?;
            assert!(
                prompt.contains(expected),
                "asked {prompt:?}, but the next answer is for {expected:?}; asked so far: {:#?}",
                self.prompts()
            );
            Some(answer.to_string())
        }

        /// Fail if `what` is to fail.
        fn fail_if(&self, what: Fails) -> Result<()> {
            if self.failing.contains(&what) {
                bail!("Permission denied (os error 13)");
            }
            Ok(())
        }
    }

    impl Outside for Scripted {
        fn has_terminal(&mut self) -> bool {
            self.terminal
        }

        fn read(&mut self, prompt: &str) -> Result<Option<String>> {
            self.calls.push(Call::Ask(prompt.to_string()));
            Ok(self.answer(prompt))
        }

        fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>> {
            self.calls.push(Call::AskHidden(prompt.to_string()));
            Ok(self.answer(prompt))
        }

        fn say(&mut self, line: String) {
            self.calls.push(Call::Say(line));
        }

        fn installed(&mut self, harness: Harness) -> bool {
            self.installed.contains(&harness)
        }

        fn test_call(&mut self, chosen: &ModelAndEffort) -> Result<()> {
            self.calls.push(Call::Tested(chosen.clone()));
            match self.test_calls.pop_front().flatten() {
                Some(refusal) => Err(anyhow!("{refusal}")),
                None => Ok(()),
            }
        }

        fn codex_catalog(&mut self) -> Result<Catalog> {
            match self.catalog {
                Ok(json) => Catalog::parse(json),
                Err(error) => bail!("{error}"),
            }
        }

        fn agy_catalog(&mut self) -> Result<crate::harness::agy::Catalog> {
            match self.agy_catalog {
                Ok(text) => crate::harness::agy::Catalog::parse(text),
                Err(error) => bail!("{error}"),
            }
        }

        fn grok_catalog(&mut self) -> Result<crate::harness::grok::Catalog> {
            match self.grok_catalog {
                Ok((list, cache)) => crate::harness::grok::Catalog::parse(list, cache),
                Err(error) => bail!("{error}"),
            }
        }

        fn muse_check(&mut self, chosen: &ModelAndEffort) -> Result<ModelAndEffort> {
            self.calls.push(Call::MuseChecked(chosen.clone()));
            self.muse_checks
                .pop_front()
                .unwrap_or_else(|| Ok(chosen.clone()))
                .map_err(|error| anyhow!("{error}"))
        }

        fn opencode_check(&mut self, _chosen: &ModelAndEffort) -> Result<()> {
            Ok(())
        }

        fn find_key(&mut self) -> Result<Option<Source>> {
            self.key.clone().map_err(|error| anyhow!("{error}"))
        }

        fn save_key(&mut self, key: &str) -> Result<String> {
            self.calls.push(Call::SaveKey(key.to_string()));
            self.fail_if(Fails::SaveKey)
                .with_context(|| format!("can't write {CREDENTIALS}"))?;
            Ok(format!("wrote the Credentials {CREDENTIALS}"))
        }

        fn github_email(&mut self) -> Result<Option<String>> {
            match self.github_email {
                Ok(email) => Ok(email.map(String::from)),
                Err(error) => bail!("{error}"),
            }
        }

        fn git_email(&mut self) -> Result<String> {
            match self.git_email {
                Ok(email) => Ok(email.to_string()),
                Err(error) => bail!("{error}"),
            }
        }

        fn read_user_config(&mut self, _: &Path) -> Result<Option<String>> {
            Ok(self.user_config.clone())
        }

        fn user_config_exists(&mut self, _: &Path) -> bool {
            self.user_config.is_some()
        }

        fn write_new_user_config(&mut self, _: &Path, text: &str) -> Result<()> {
            self.calls.push(Call::WriteNew(text.to_string()));
            self.fail_if(Fails::WriteNew)?;
            self.user_config = Some(text.to_string());
            Ok(())
        }

        fn replace_user_config(&mut self, _: &Path, text: &str) -> Result<()> {
            self.calls.push(Call::Replace(text.to_string()));
            self.fail_if(Fails::Replace)?;
            self.user_config = Some(text.to_string());
            Ok(())
        }

        fn send_test(&mut self, email: &EmailSettings) -> Result<String> {
            self.calls.push(Call::SendTest {
                to: email.to.clone(),
                from: email.from.clone(),
            });
            self.fail_if(Fails::SendTest)?;
            Ok(SENT.to_string())
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::scripted::{CREDENTIALS, Call, Fails, SENT, Scripted};
    use super::*;
    use crate::config::DEFAULTS;

    const HOME: &str = "/home/me";
    const PATH: &str = "/home/me/.thirdshift/config.toml";

    const OFFER: &str =
        "No User config at /home/me/.thirdshift/config.toml. Set your defaults now? [Y/n] ";
    const HARNESS: &str = "Harness for every Run's sessions";
    const MODEL: &str = "Model for claude";
    const EFFORT: &str = "Effort for claude";
    const CODEX_MODEL: &str = "Model for codex";
    const CODEX_EFFORT: &str = "Effort for codex";
    const MERGE: &str = "Every Run a Merge run?";
    const BASE_FIX: &str = "Every Run may start a Base fix when the Base branch's CI is red?";
    const PULL: &str = "fast-forward";
    const NOTIFY: &str = "Run notifications, an email";
    const TO: &str = "Send Run notifications to";
    const FROM: &str = "Send them from";
    const TEST_EMAIL: &str = "Send a test email now? [y/N] ";
    const KEY_QUESTION: &str = "Resend API key (input hidden";
    const KEY_PROMPT: &str = "Resend API key (input hidden, Enter to skip): ";
    const KEPT: &str = "Resend API key (input hidden, Enter keeps the saved one): ";
    const SKIPPED: &str = "No Resend API key, so no email can go yet. To add one later, rerun \
                           `thirdshift setup`, or set RESEND_API_KEY in the environment the Run \
                           starts from.";
    const ENDED: &str = "Setup ended before its last answer; nothing written";
    const KEY: &str = "re_secret_123";

    /// The answers that take every default and turn nothing on.
    const ENTER_THROUGHOUT: [(&str, &str); 6] = [
        (HARNESS, ""),
        (MODEL, ""),
        (EFFORT, ""),
        (MERGE, ""),
        (PULL, ""),
        (NOTIFY, ""),
    ];

    /// The answers after the Harness's that leave every setting as it is.
    const NOTHING_ELSE: [(&str, &str); 3] = [(MERGE, ""), (PULL, ""), (NOTIFY, "")];

    /// The answers that turn Run notifications on, to `me@example.com` from
    /// the default sender, then `rest`.
    fn notifications_on(
        rest: &[(&'static str, &'static str)],
    ) -> Vec<(&'static str, &'static str)> {
        let mut answers = vec![
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, ""),
        ];
        answers.extend_from_slice(rest);
        answers
    }

    /// `harness` answers, then [`NOTHING_ELSE`].
    fn then_nothing_else(
        harness: &[(&'static str, &'static str)],
    ) -> Vec<(&'static str, &'static str)> {
        let mut answers = harness.to_vec();
        answers.extend(NOTHING_ELSE);
        answers
    }

    /// Run Setup, and check it took every answer it was given.
    fn setup(outside: &mut Scripted) -> Result<String> {
        let done = run_setup(outside, Path::new(HOME), Path::new(PATH));
        if done.is_ok() {
            assert!(outside.answers.is_empty(), "unasked: {:?}", outside.answers);
        }
        done
    }

    /// Make the first Run's offer of Setup, and check it took every answer
    /// it was given.
    fn offer(outside: &mut Scripted) -> Result<()> {
        let done = run_offer(outside, Path::new(HOME), Path::new(PATH));
        if done.is_ok() {
            assert!(outside.answers.is_empty(), "unasked: {:?}", outside.answers);
        }
        done
    }

    /// The User config as written, parsed.
    fn table(outside: &Scripted) -> toml::Table {
        let text = outside.user_config.as_deref().expect("no User config");
        text.parse().unwrap()
    }

    /// Whether anything was written, saved or sent.
    fn wrote_anything(outside: &Scripted) -> bool {
        outside.calls.iter().any(|call| {
            matches!(
                call,
                Call::WriteNew(_) | Call::Replace(_) | Call::SaveKey(_) | Call::SendTest { .. }
            )
        })
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    fn model_and_effort(model: Option<&str>, effort: Option<&str>) -> ModelAndEffort {
        ModelAndEffort {
            model: model.map(String::from),
            effort: effort.map(String::from),
        }
    }

    // What Setup writes.

    #[test]
    fn with_no_terminal_and_no_user_config_setup_writes_the_defaults() {
        let mut outside = Scripted::unattended();

        let done = setup(&mut outside).unwrap();

        assert_eq!(done, format!("wrote the User config {PATH}"));
        assert_eq!(outside.effects(), [&Call::WriteNew(DEFAULTS.to_string())]);
        assert!(outside.prompts().is_empty());
    }

    #[test]
    fn with_no_terminal_the_credentials_are_never_read_or_written() {
        let mut outside = Scripted {
            key: Err("unknown key resend.kye in the Credentials"),
            ..Scripted::unattended()
        };

        setup(&mut outside).unwrap();

        assert_eq!(outside.effects(), [&Call::WriteNew(DEFAULTS.to_string())]);
    }

    #[test]
    fn setup_over_a_partial_user_config_keeps_it_and_adds_each_missing_key() {
        let partial = "\
# My machine: always merge.
[merge]
always = true   # I trust the factory

[email]
to = \"me@example.com\"  # my inbox
";
        let mut outside = Scripted {
            user_config: Some(partial.to_string()),
            ..Scripted::unattended()
        };

        let done = setup(&mut outside).unwrap();

        assert_eq!(
            done,
            format!("added the missing settings to the User config {PATH}")
        );
        let completed = config::complete(partial).unwrap();
        assert_eq!(outside.effects(), [&Call::Replace(completed.clone())]);
        assert!(
            completed.starts_with(
                "# My machine: always merge.\n[merge]\nalways = true   # I trust the factory\n"
            ),
            "{completed}"
        );
        assert!(
            completed.contains("to = \"me@example.com\"  # my inbox\n"),
            "{completed}"
        );
        assert!(
            completed.contains(
                "\n[pickup]\nlimit = 3   # how many open issues labelled in-progress stop a \
                 Pickup run taking another; default 3\n"
            ),
            "{completed}"
        );
    }

    #[test]
    fn setup_over_a_complete_user_config_leaves_it_as_it_was() {
        let edited = DEFAULTS
            .replace("always = false   #", "always = true    #")
            .replace("\"~/.thirdshift/logs\"", "\"/var/log/thirdshift\"")
            + "# hand-written at the end\n";
        let mut outside = Scripted {
            user_config: Some(edited.clone()),
            ..Scripted::unattended()
        };

        let done = setup(&mut outside).unwrap();

        assert_eq!(
            done,
            format!("the User config {PATH} already lists every setting")
        );
        assert!(outside.effects().is_empty());
        assert_eq!(outside.user_config, Some(edited));
    }

    #[test]
    fn setup_refuses_a_user_config_a_run_would_refuse_before_asking_and_leaves_it_alone() {
        for (broken, named) in [
            ("[merge]\nalway = true\n", "unknown key merge.alway"),
            ("[merge\nalways = true\n", "can't parse the User config"),
            (
                "# mine\n[launch]\npull = \"yes\"\n",
                "launch.pull must be true or false",
            ),
        ] {
            for terminal in [false, true] {
                let mut outside = Scripted {
                    terminal,
                    user_config: Some(broken.to_string()),
                    ..Scripted::answering(&ENTER_THROUGHOUT)
                };

                let error = format!("{:#}", setup(&mut outside).unwrap_err());

                let refused = UserConfig::parse(broken, Path::new(PATH), Path::new(HOME));
                assert_eq!(error, format!("{:#}", refused.unwrap_err()));
                for part in [named, PATH] {
                    assert!(error.contains(part), "{broken:?}: {error}");
                }
                assert!(outside.prompts().is_empty(), "{broken:?}");
                assert!(outside.effects().is_empty(), "{broken:?}");
            }
        }
    }

    #[test]
    fn on_a_terminal_pressing_enter_throughout_writes_what_setup_with_no_terminal_writes() {
        let mut unattended = Scripted::unattended();
        setup(&mut unattended).unwrap();
        let mut outside = Scripted::answering(&ENTER_THROUGHOUT);

        let done = setup(&mut outside).unwrap();

        assert_eq!(done, format!("wrote the User config {PATH}"));
        assert_eq!(outside.user_config, unattended.user_config);
        assert_eq!(
            outside.prompts(),
            [
                "Harness for every Run's sessions, claude or codex or agy or grok or muse or opencode [claude]: ",
                "Model for claude [claude's own default]: ",
                "Effort for claude [claude's own default]: ",
                "Every Run a Merge run? [y/N] ",
                "Every Run first fast-forwards your checkout of the Base branch? [y/N] ",
                "Run notifications, an email as each Run ends? [y/N] ",
            ]
        );
    }

    #[test]
    fn on_a_terminal_the_answers_are_written_with_the_comments_on_each_key() {
        let mut outside = Scripted {
            key: Ok(Some(Source::Environment)),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "y"),
                (BASE_FIX, "y"),
                (PULL, "yes"),
                (NOTIFY, "y"),
                (TO, "me@example.com"),
                (FROM, "ts@acme.dev"),
                (TEST_EMAIL, ""),
            ])
        };

        setup(&mut outside).unwrap();

        let config = table(&outside);
        assert_eq!(config["merge"]["always"].as_bool(), Some(true));
        assert_eq!(config["base"]["fix"].as_bool(), Some(true));
        assert_eq!(config["launch"]["pull"].as_bool(), Some(true));
        assert_eq!(config["email"]["always"].as_bool(), Some(true));
        assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
        assert_eq!(config["email"]["from"].as_str(), Some("ts@acme.dev"));
        let text = outside.user_config.unwrap();
        let expected = DEFAULTS
            .replace(
                "always = false   # every Run is a Merge",
                "always = true    # every Run is a Merge",
            )
            .replace("fix = false      #", "fix = true       #")
            .replace("pull = false     #", "pull = true      #")
            .replace(
                "always = false                  #",
                "always = true                   #",
            )
            .replace(
                "# to = \"you@example.com\"        #",
                "to = \"me@example.com\"           #",
            )
            .replace(
                "from = \"onboarding@resend.dev\"  #",
                "from = \"ts@acme.dev\"            #",
            );
        assert_eq!(text, expected);
    }

    #[test]
    fn on_a_terminal_pressing_enter_throughout_keeps_an_existing_user_config() {
        let mine = "\
[merge]
always = true

[base]
fix = true

[launch]
pull = true

[email]
always = true
to = \"mine@example.net\"
from = \"ts@acme.dev\"

[logs]
dir = \"/var/log/thirdshift\"

[activity]
quiet_skips = true

[spec]
parallel = 5

[pickup]
limit = 5

[harness]
default = \"claude\"

[harness.claude]
model = \"opus\"
effort = \"high\"

[harness.codex]
model = \"gpt-6.1-sol\"
effort = \"max\"

[harness.agy]
model = \"\"
effort = \"\"

[harness.grok]
model = \"\"
effort = \"\"

[harness.muse]
model = \"\"
effort = \"\"

[harness.opencode]
model = \"\"
effort = \"\"
";
        let mut outside = Scripted {
            user_config: Some(mine.to_string()),
            key: Ok(Some(Source::Environment)),
            github_email: Ok(Some("octo@example.com")),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (BASE_FIX, ""),
                (PULL, ""),
                (NOTIFY, ""),
                (TO, ""),
                (FROM, ""),
                (TEST_EMAIL, ""),
            ])
        };

        let done = setup(&mut outside).unwrap();

        assert_eq!(
            done,
            format!("the User config {PATH} already holds your answers")
        );
        assert!(outside.effects().is_empty());
        assert_eq!(
            outside.prompts(),
            [
                "Harness for every Run's sessions, claude or codex or agy or grok or muse or opencode [claude]: ",
                "Model for claude, - for claude's own default [opus]: ",
                "Effort for claude, - for claude's own default [high]: ",
                "Every Run a Merge run? [Y/n] ",
                "Every Run may start a Base fix when the Base branch's CI is red? [Y/n] ",
                "Every Run first fast-forwards your checkout of the Base branch? [Y/n] ",
                "Run notifications, an email as each Run ends? [Y/n] ",
                "Send Run notifications to [mine@example.net]: ",
                "Send them from [ts@acme.dev]: ",
                "Send a test email now? [y/N] ",
            ]
        );
    }

    #[test]
    fn on_a_terminal_changed_answers_are_written_over_the_user_config() {
        let mut outside = Scripted {
            user_config: Some(DEFAULTS.to_string()),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, "y"),
                (NOTIFY, ""),
            ])
        };

        let done = setup(&mut outside).unwrap();

        assert_eq!(
            done,
            format!("wrote your answers to the User config {PATH}")
        );
        let answered = DEFAULTS.replace("pull = false     #", "pull = true      #");
        assert_eq!(outside.effects(), [&Call::Replace(answered)]);
    }

    #[test]
    fn on_a_terminal_setup_over_a_hand_commented_user_config_keeps_the_comments() {
        let mine = "\
# My machine.
[merge]
# Merging is for later.
always = false   # not yet

[email]
# to = \"someday@example.com\"
always = false # quiet, please
";
        let mut outside = Scripted {
            user_config: Some(mine.to_string()),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "y"),
                (BASE_FIX, ""),
                (PULL, ""),
                (NOTIFY, "y"),
                (TO, "me@example.com"),
                (FROM, ""),
                (KEY_PROMPT, ""),
            ])
        };

        setup(&mut outside).unwrap();

        let text = outside.user_config.clone().unwrap();
        for kept in [
            "# My machine.\n[merge]\n# Merging is for later.\nalways = true    # not yet\n",
            "always = true  # quiet, please\n",
        ] {
            assert!(text.contains(kept), "{text}");
        }
        assert!(!text.contains("# to = "), "{text}");
        let config = table(&outside);
        assert_eq!(config["merge"]["always"].as_bool(), Some(true));
        assert_eq!(config["email"]["to"].as_str(), Some("me@example.com"));
    }

    #[test]
    fn the_credentials_are_saved_after_the_user_config_and_the_test_email_goes_last() {
        let mut outside =
            Scripted::answering(&notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "y")]));

        let done = setup(&mut outside).unwrap();

        assert_eq!(done, SENT);
        assert_eq!(
            outside.effects()[1..],
            [
                &step(&format!("wrote the User config {PATH}")),
                &Call::SaveKey(KEY.to_string()),
                &step(&format!("wrote the Credentials {CREDENTIALS}")),
                &Call::SendTest {
                    to: Some("me@example.com".to_string()),
                    from: Some(crate::email::DEFAULT_FROM.to_string()),
                },
            ]
        );
        assert!(matches!(outside.effects()[0], Call::WriteNew(_)));
    }

    #[test]
    fn an_entered_key_alone_ends_with_the_line_saying_where_it_was_saved() {
        let mut outside =
            Scripted::answering(&notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "n")]));

        let done = setup(&mut outside).unwrap();

        assert_eq!(done, format!("wrote the Credentials {CREDENTIALS}"));
        assert!(!outside.user_config.unwrap().contains(KEY));
    }

    #[test]
    fn stdin_closing_before_the_last_answer_writes_nothing() {
        for answers in [
            vec![],
            vec![
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "y"),
                (BASE_FIX, ""),
            ],
            notifications_on(&[]),
            notifications_on(&[(KEY_PROMPT, KEY)]),
        ] {
            for existing in [None, Some("[launch]\npull = true # mine\n")] {
                let mut outside = Scripted {
                    user_config: existing.map(String::from),
                    ..Scripted::answering(&answers)
                };

                let error = setup(&mut outside).unwrap_err();

                assert_eq!(error.to_string(), ENDED, "{answers:?}");
                assert!(!wrote_anything(&outside), "{answers:?}");
                assert_eq!(outside.user_config.as_deref(), existing);
            }
        }
    }

    #[test]
    fn on_a_terminal_setup_refuses_broken_credentials_before_asking() {
        let mut outside = Scripted {
            key: Err("unknown key resend.kye in the Credentials"),
            ..Scripted::answering(&ENTER_THROUGHOUT)
        };

        let error = setup(&mut outside).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unknown key resend.kye in the Credentials"
        );
        assert!(outside.prompts().is_empty());
        assert!(outside.effects().is_empty());
    }

    // The questions.

    #[test]
    fn with_merging_on_setup_asks_about_base_fixes_and_writes_the_answer() {
        for (answer, allowed) in [("y", true), ("", false), ("n", false)] {
            let mut outside = Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "y"),
                (BASE_FIX, answer),
                (PULL, ""),
                (NOTIFY, ""),
            ]);

            setup(&mut outside).unwrap();

            let config = table(&outside);
            assert_eq!(config["merge"]["always"].as_bool(), Some(true));
            assert_eq!(config["base"]["fix"].as_bool(), Some(allowed), "{answer:?}");
            assert!(
                outside
                    .prompts()
                    .contains(&format!("{BASE_FIX} [y/N] ").as_str()),
                "{:?}",
                outside.prompts()
            );
        }
    }

    #[test]
    fn with_merging_off_setup_asks_nothing_about_base_fixes_and_writes_the_default() {
        for answer in ["", "n"] {
            let mut outside = Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, answer),
                (PULL, ""),
                (NOTIFY, ""),
            ]);

            setup(&mut outside).unwrap();

            let config = table(&outside);
            assert_eq!(config["merge"]["always"].as_bool(), Some(false));
            assert_eq!(config["base"]["fix"].as_bool(), Some(false));
        }
    }

    #[test]
    fn the_base_fix_question_defaults_to_the_user_configs_base_fix() {
        let mut outside = Scripted {
            user_config: Some("[merge]\nalways = true\n\n[base]\nfix = true # mine\n".to_string()),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (BASE_FIX, ""),
                (PULL, ""),
                (NOTIFY, ""),
            ])
        };

        setup(&mut outside).unwrap();

        assert!(
            outside
                .prompts()
                .contains(&format!("{BASE_FIX} [Y/n] ").as_str()),
            "{:?}",
            outside.prompts()
        );
        let text = outside.user_config.unwrap();
        assert!(text.contains("fix = true # mine\n"), "{text}");
    }

    #[test]
    fn turning_merging_off_writes_base_fix_at_its_default() {
        let mut outside = Scripted {
            user_config: Some("[merge]\nalways = true\n\n[base]\nfix = true  # mine\n".to_string()),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, "n"),
                (PULL, ""),
                (NOTIFY, ""),
            ])
        };

        setup(&mut outside).unwrap();

        let text = outside.user_config.clone().unwrap();
        assert!(text.contains("fix = false # mine\n"), "{text}");
        assert_eq!(table(&outside)["merge"]["always"].as_bool(), Some(false));
    }

    #[test]
    fn a_yes_or_no_question_is_asked_again_until_the_answer_is_one() {
        let mut outside = Scripted::answering(&[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "maybe"),
            (MERGE, "YES"),
            (BASE_FIX, "No"),
            (PULL, ""),
            (NOTIFY, ""),
        ]);

        setup(&mut outside).unwrap();

        assert_eq!(table(&outside)["merge"]["always"].as_bool(), Some(true));
    }

    #[test]
    fn an_address_without_an_at_is_asked_again() {
        let mut outside = Scripted::answering(&[
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, ""),
            (TO, "me.example.com"),
            (TO, "me@example.com"),
            (FROM, ""),
            (KEY_PROMPT, ""),
        ]);

        setup(&mut outside).unwrap();

        let asked = outside.prompts();
        assert_eq!(
            asked.iter().filter(|prompt| prompt.contains(TO)).count(),
            3,
            "{asked:?}"
        );
        assert!(asked.contains(&"Send Run notifications to: "), "{asked:?}");
        assert_eq!(
            table(&outside)["email"]["to"].as_str(),
            Some("me@example.com")
        );
    }

    #[test]
    fn with_notifications_off_nothing_about_email_is_asked() {
        let mut outside = Scripted {
            key: Ok(Some(Source::Credentials(CREDENTIALS.into()))),
            github_email: Ok(Some("octo@example.com")),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (NOTIFY, "n"),
            ])
        };

        setup(&mut outside).unwrap();

        for prompt in outside.prompts() {
            for asked in [TO, FROM, KEY_QUESTION, TEST_EMAIL] {
                assert!(!prompt.contains(asked), "{prompt}");
            }
        }
        assert!(outside.said().is_empty(), "{:?}", outside.said());
        assert_eq!(outside.effects().len(), 1, "{:?}", outside.effects());
        assert_eq!(table(&outside)["email"]["always"].as_bool(), Some(false));
    }

    #[test]
    fn with_no_key_anywhere_skipping_it_saves_nothing_offers_no_test_email_and_says_how_to_add_one()
    {
        let mut outside = Scripted::answering(&notifications_on(&[(KEY_PROMPT, "")]));

        let done = setup(&mut outside).unwrap();

        assert_eq!(done, format!("wrote the User config {PATH}"));
        assert_eq!(outside.said(), [SKIPPED]);
        assert!(!outside.prompts().contains(&TEST_EMAIL));
        assert_eq!(outside.effects().len(), 1, "{:?}", outside.effects());
        assert_eq!(table(&outside)["email"]["always"].as_bool(), Some(true));
    }

    #[test]
    fn the_key_prompt_is_hidden_and_a_key_not_starting_with_re_is_asked_again() {
        let mut outside = Scripted::answering(&notifications_on(&[
            (KEY_PROMPT, "sk_not_resend"),
            (KEY_PROMPT, KEY),
            (TEST_EMAIL, ""),
        ]));

        setup(&mut outside).unwrap();

        let hidden: Vec<&Call> = outside
            .calls
            .iter()
            .filter(|call| matches!(call, Call::AskHidden(_)))
            .collect();
        assert_eq!(hidden, [&Call::AskHidden(KEY_PROMPT.to_string()); 2]);
        assert_eq!(
            outside.said(),
            ["That isn't a Resend API key, which starts with re_."]
        );
        assert!(outside.calls.contains(&Call::SaveKey(KEY.to_string())));
        assert!(
            !outside
                .calls
                .contains(&Call::SaveKey("sk_not_resend".to_string()))
        );
    }

    #[test]
    fn with_a_saved_key_pressing_enter_keeps_it() {
        let mut outside = Scripted {
            key: Ok(Some(Source::Credentials(CREDENTIALS.into()))),
            ..Scripted::answering(&notifications_on(&[(KEPT, ""), (TEST_EMAIL, "")]))
        };

        setup(&mut outside).unwrap();

        assert!(outside.said().is_empty(), "{:?}", outside.said());
        assert!(
            !outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::SaveKey(_)))
        );
    }

    #[test]
    fn with_a_saved_key_a_new_one_replaces_it() {
        let mut outside = Scripted {
            key: Ok(Some(Source::Credentials(CREDENTIALS.into()))),
            ..Scripted::answering(&notifications_on(&[(KEPT, KEY), (TEST_EMAIL, "")]))
        };

        setup(&mut outside).unwrap();

        assert!(outside.calls.contains(&Call::SaveKey(KEY.to_string())));
    }

    #[test]
    fn with_resend_api_key_set_no_key_is_asked_and_its_source_is_said() {
        let mut outside = Scripted {
            key: Ok(Some(Source::Environment)),
            ..Scripted::answering(&notifications_on(&[(TEST_EMAIL, "")]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.said(),
            ["The Resend API key comes from RESEND_API_KEY."]
        );
        for prompt in outside.prompts() {
            assert!(!prompt.contains(KEY_QUESTION), "{prompt}");
        }
    }

    #[test]
    fn accepting_the_test_email_sends_one_and_declining_sends_none() {
        for (answer, sent) in [("y", true), ("n", false), ("", false)] {
            let mut outside = Scripted {
                key: Ok(Some(Source::Environment)),
                ..Scripted::answering(&notifications_on(&[(TEST_EMAIL, answer)]))
            };

            let done = setup(&mut outside).unwrap();

            let sends: Vec<&Call> = outside
                .calls
                .iter()
                .filter(|call| matches!(call, Call::SendTest { .. }))
                .collect();
            if sent {
                assert_eq!(done, SENT);
                assert_eq!(
                    sends,
                    [&Call::SendTest {
                        to: Some("me@example.com".to_string()),
                        from: Some(crate::email::DEFAULT_FROM.to_string()),
                    }]
                );
            } else {
                assert!(sends.is_empty(), "{answer:?}");
            }
            assert!(outside.user_config.is_some());
        }
    }

    #[test]
    fn muse_setup_proposes_its_non_contributor_model_and_retries_a_refused_check() {
        let mut outside = Scripted {
            installed: vec![Harness::Muse],
            muse_checks: [
                Err("Muse refused the Model"),
                Ok(ModelAndEffort {
                    model: Some("muse-spark-1.3".to_string()),
                    effort: Some("high".to_string()),
                }),
            ]
            .into(),
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, "muse"),
                ("Model for muse", "bad-model"),
                ("Effort for muse", "low"),
                ("Model for muse", ""),
                ("Effort for muse", "High"),
            ]))
        };
        setup(&mut outside).unwrap();
        assert_eq!(
            outside.prompts()[1],
            "Model for muse, - for muse's own default [muse-spark-1.3]: "
        );
        assert!(outside.said().contains(&"Muse refused the Model"));
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("muse"));
        assert_eq!(
            config["harness"]["muse"]["model"].as_str(),
            Some("muse-spark-1.3")
        );
        assert_eq!(config["harness"]["muse"]["effort"].as_str(), Some("high"));
    }

    // The Harness, Model and Effort.

    #[test]
    fn the_harness_question_lists_installed_harnesses_and_defaults_to_claude() {
        let mut outside = Scripted::answering(&ENTER_THROUGHOUT);

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.prompts()[0],
            "Harness for every Run's sessions, claude or codex or agy or grok or muse or opencode [claude]: "
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
        for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
            for key in ["model", "effort"] {
                assert_eq!(config["harness"][harness][key].as_str(), Some(""));
            }
        }
        assert!(outside.test_calls().is_empty());
    }

    #[test]
    fn a_harness_not_installed_or_unknown_is_refused_and_asked_again() {
        let mut outside = Scripted {
            installed: vec![Harness::Codex],
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, "claude"),
                (HARNESS, "gemini"),
                (HARNESS, "codex"),
                (CODEX_MODEL, ""),
                (CODEX_EFFORT, ""),
            ]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.prompts()[0],
            "Harness for every Run's sessions, codex [codex]: "
        );
        assert_eq!(
            outside.said()[..2],
            [
                "claude is not installed: it isn't on PATH.",
                "Choose codex."
            ]
        );
        assert_eq!(
            table(&outside)["harness"]["default"].as_str(),
            Some("codex")
        );
    }

    #[test]
    fn with_no_harness_installed_the_harness_settings_are_left_as_they_are() {
        let mut outside = Scripted {
            installed: Vec::new(),
            user_config: Some("[harness.claude]\nmodel = \"opus\"\n".to_string()),
            ..Scripted::answering(&NOTHING_ELSE)
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.said(),
            [
                "No Harness is installed here, so the harness settings stay as they are; \
                 install claude or codex or agy or grok or muse or opencode, then rerun `thirdshift setup`."
            ]
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
        assert_eq!(config["harness"]["claude"]["model"].as_str(), Some("opus"));
    }

    #[test]
    fn the_harness_default_is_the_configured_one_if_installed_else_the_first_installed() {
        for (installed, default) in [
            (vec![Harness::Claude, Harness::Codex], "codex"),
            (vec![Harness::Claude], "claude"),
        ] {
            let mut outside = Scripted {
                installed,
                user_config: Some("[harness]\ndefault = \"codex\"\n".to_string()),
                ..Scripted::answering(&[(HARNESS, "")])
            };

            setup(&mut outside).unwrap_err();

            assert!(
                outside.prompts()[0].ends_with(&format!(" [{default}]: ")),
                "{:?}",
                outside.prompts()
            );
        }
    }

    #[test]
    fn agy_setup_checks_names_without_a_turn_and_lists_only_installed_harnesses() {
        let mut outside = Scripted {
            installed: vec![Harness::Agy],
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, ""),
                ("Model for agy", "gemini-99"),
                ("Model for agy", "Gemini-3.8-Flash"),
                ("Effort for agy", "Max"),
                ("Effort for agy", "Medium"),
            ]))
        };
        setup(&mut outside).unwrap();
        assert_eq!(
            outside.prompts()[0],
            "Harness for every Run's sessions, agy [agy]: "
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("agy"));
        assert_eq!(
            config["harness"]["agy"]["model"].as_str(),
            Some("gemini-3.8-flash")
        );
        assert_eq!(config["harness"]["agy"]["effort"].as_str(), Some("medium"));
        assert!(
            outside
                .said()
                .iter()
                .any(|line| line.contains("the Model gemini-99 is not in"))
        );
        assert!(
            outside
                .said()
                .iter()
                .any(|line| line.contains("the Effort Max is not one"))
        );
        assert!(outside.test_calls().is_empty());
    }

    #[test]
    fn an_unreadable_agy_catalog_keeps_existing_harness_settings() {
        let mut outside = Scripted {
            installed: vec![Harness::Agy],
            agy_catalog: Err("not signed in"),
            user_config: Some(
                "[harness]\ndefault = \"agy\"\n[harness.agy]\nmodel = \"gemini-3.8-flash-high\"\n"
                    .to_string(),
            ),
            ..Scripted::answering(&then_nothing_else(&[(HARNESS, "")]))
        };
        setup(&mut outside).unwrap();
        assert_eq!(
            table(&outside)["harness"]["agy"]["model"].as_str(),
            Some("gemini-3.8-flash-high")
        );
        assert!(outside.said()[0].contains("not signed in"));
        assert!(outside.said()[0].contains("once agy models works"));
    }

    #[test]
    fn claudes_model_and_effort_are_written_after_a_test_call_with_them() {
        let mut outside = Scripted::answering(&then_nothing_else(&[
            (HARNESS, ""),
            (MODEL, "claude-opus-5-5"),
            (EFFORT, "high"),
        ]));

        setup(&mut outside).unwrap();

        let config = table(&outside);
        assert_eq!(
            config["harness"]["claude"]["model"].as_str(),
            Some("claude-opus-5-5")
        );
        assert_eq!(config["harness"]["claude"]["effort"].as_str(), Some("high"));
        assert_eq!(config["harness"]["codex"]["model"].as_str(), Some(""));
        let text = outside.user_config.clone().unwrap();
        assert!(
            text.contains("model = \"claude-opus-5-5\" # the Model Claude Code's sessions run on"),
            "{text}"
        );
        assert_eq!(
            outside.test_calls(),
            [&model_and_effort(Some("claude-opus-5-5"), Some("high"))]
        );
    }

    #[test]
    fn the_model_and_effort_default_to_the_current_ones_and_a_dash_clears_one() {
        let mut outside = Scripted {
            user_config: Some(
                "[harness.claude]\nmodel = \"opus\"   # mine\neffort = \"high\"\n\n\
                 [harness.codex]\nmodel = \"gpt-6.1-sol\"\n"
                    .to_string(),
            ),
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, "-"),
            ]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.prompts()[1..3],
            [
                "Model for claude, - for claude's own default [opus]: ",
                "Effort for claude, - for claude's own default [high]: ",
            ]
        );
        let text = outside.user_config.clone().unwrap();
        assert!(text.contains("model = \"opus\"   # mine\n"), "{text}");
        let config = table(&outside);
        assert_eq!(config["harness"]["claude"]["effort"].as_str(), Some(""));
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(
            outside.test_calls(),
            [&model_and_effort(Some("opus"), None)]
        );
    }

    #[test]
    fn a_model_claude_refuses_is_asked_again_with_claudes_error() {
        let refusal = "claude refused a test call on the Model Opus 5.5: There's an issue with \
                       the selected model (Opus 5.5).";
        let mut outside = Scripted {
            test_calls: [Some(refusal)].into(),
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, ""),
                (MODEL, "Opus 5.5"),
                (EFFORT, ""),
                (MODEL, "opus"),
                (EFFORT, ""),
            ]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(outside.said(), [refusal]);
        assert_eq!(
            table(&outside)["harness"]["claude"]["model"].as_str(),
            Some("opus")
        );
        assert_eq!(
            outside.test_calls(),
            [
                &model_and_effort(Some("Opus 5.5"), None),
                &model_and_effort(Some("opus"), None)
            ]
        );
    }

    #[test]
    fn setup_offers_grok_when_it_is_the_only_installed_harness_and_checks_its_model_and_effort() {
        let mut outside = Scripted {
            installed: vec![Harness::Grok],
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, ""),
                ("Model for grok", "bogus"),
                ("Model for grok", "GROK-4.5"),
                ("Effort for grok", "xhigh"),
                ("Effort for grok", "High"),
            ]))
        };

        setup(&mut outside).unwrap();

        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("grok"));
        assert_eq!(
            config["harness"]["grok"]["model"].as_str(),
            Some("grok-4.5")
        );
        assert_eq!(config["harness"]["grok"]["effort"].as_str(), Some("high"));
        let said = outside.said().join("\n");
        assert!(
            said.contains("choose one of grok-4.7, grok-4.7-build-fast, grok-4.6, grok-4.5"),
            "{said}"
        );
        assert!(said.contains("choose one of high, medium, low"), "{said}");
        assert!(outside.test_calls().is_empty());
    }

    #[test]
    fn when_groks_catalog_cannot_be_read_setup_preserves_the_harness_settings() {
        let mut outside = Scripted {
            grok_catalog: Err("catalog unavailable"),
            user_config: Some("[harness]\ndefault = \"codex\"\n[harness.grok]\nmodel = \"grok-4.5\"\neffort = \"medium\"\n".to_string()),
            ..Scripted::answering(&then_nothing_else(&[(HARNESS, "grok")]))
        };
        setup(&mut outside).unwrap();
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
        assert_eq!(
            config["harness"]["grok"]["model"].as_str(),
            Some("grok-4.5")
        );
        assert_eq!(config["harness"]["grok"]["effort"].as_str(), Some("medium"));
        assert!(outside.said().join("\n").contains("once grok models works"));
    }

    #[test]
    fn codex_is_offered_with_its_models_and_the_chosen_models_efforts() {
        let mut outside = Scripted::answering(&then_nothing_else(&[
            (HARNESS, "codex"),
            (CODEX_MODEL, "gpt-5.5"),
            (CODEX_EFFORT, "high"),
        ]));

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.said(),
            [
                "Codex's Models: gpt-6.1-sol (GPT-6.1-Sol), gpt-6-luna (GPT-6-Luna), gpt-5.5 \
                 (GPT-5.5)",
                "Efforts gpt-5.5 supports: low, medium, high, xhigh",
            ]
        );
        assert_eq!(
            outside.prompts()[1..3],
            [
                "Model for codex [codex's own default]: ",
                "Effort for codex [codex's own default]: ",
            ]
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-5.5")
        );
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("high"));
        assert_eq!(config["harness"]["claude"]["model"].as_str(), Some(""));
        assert!(outside.test_calls().is_empty());
    }

    #[test]
    fn with_no_codex_model_the_efforts_any_model_supports_are_listed() {
        let mut outside = Scripted {
            installed: vec![Harness::Codex],
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, ""),
                (CODEX_MODEL, ""),
                (CODEX_EFFORT, ""),
            ]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.said()[1],
            "Efforts Codex's Models support: low, medium, high, xhigh, max, ultra"
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("codex"));
        assert_eq!(config["harness"]["codex"]["model"].as_str(), Some(""));
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some(""));
    }

    #[test]
    fn a_codex_display_name_and_capitalised_effort_are_written_as_codex_names_them() {
        let mut outside = Scripted::answering(&then_nothing_else(&[
            (HARNESS, "codex"),
            (CODEX_MODEL, "GPT-6.1-Sol"),
            (CODEX_EFFORT, "Max"),
        ]));

        setup(&mut outside).unwrap();

        let config = table(&outside);
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-6.1-sol")
        );
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("max"));
    }

    #[test]
    fn an_unknown_codex_model_or_unsupported_effort_is_asked_again_with_the_choices() {
        let mut outside = Scripted::answering(&then_nothing_else(&[
            (HARNESS, "codex"),
            (CODEX_MODEL, "gpt-7"),
            (CODEX_MODEL, "GPT-5.5"),
            (CODEX_EFFORT, "max"),
            (CODEX_EFFORT, "XHigh"),
        ]));

        setup(&mut outside).unwrap();

        let said = outside.said();
        for refusal in [
            "the Model gpt-7 is not in Codex's catalog: choose one of gpt-6.1-sol, gpt-6-luna, \
             gpt-5.5",
            "the Effort max is not one the Codex Model gpt-5.5 supports: choose one of low, \
             medium, high, xhigh",
        ] {
            assert!(said.contains(&refusal), "{said:#?}");
        }
        let config = table(&outside);
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-5.5")
        );
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("xhigh"));
    }

    #[test]
    fn the_codex_model_and_effort_default_to_the_current_ones_as_codex_names_them() {
        let mut outside = Scripted {
            user_config: Some(
                "[harness.codex]\nmodel = \"GPT-6-Luna\"\neffort = \"High\"\n".to_string(),
            ),
            ..Scripted::answering(&then_nothing_else(&[
                (HARNESS, "codex"),
                (CODEX_MODEL, ""),
                (CODEX_EFFORT, ""),
            ]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.prompts()[1..3],
            [
                "Model for codex, - for codex's own default [GPT-6-Luna]: ",
                "Effort for codex, - for codex's own default [High]: ",
            ]
        );
        assert_eq!(
            outside.said()[1],
            "Efforts gpt-6-luna supports: low, medium, high, xhigh, max"
        );
        let config = table(&outside);
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-6-luna")
        );
        assert_eq!(config["harness"]["codex"]["effort"].as_str(), Some("high"));
    }

    #[test]
    fn when_codexs_catalog_cant_be_read_the_harness_settings_are_left_as_they_are() {
        let mut outside = Scripted {
            catalog: Err(
                "codex debug models failed, so Codex's Models can't be read: not logged in",
            ),
            user_config: Some("[harness.codex]\nmodel = \"gpt-5.5\"\n".to_string()),
            ..Scripted::answering(&then_nothing_else(&[(HARNESS, "codex")]))
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.said(),
            [
                "codex debug models failed, so Codex's Models can't be read: not logged in\n\
                 The harness settings stay as they are; rerun `thirdshift setup` once codex \
                 debug models works."
            ]
        );
        let config = table(&outside);
        assert_eq!(config["harness"]["default"].as_str(), Some("claude"));
        assert_eq!(
            config["harness"]["codex"]["model"].as_str(),
            Some("gpt-5.5")
        );
    }

    // The suggested address.

    /// The `email.to` Setup with no terminal writes, with `github` and `git`
    /// the two emails it can suggest.
    fn suggested(
        github: Result<Option<&'static str>, &'static str>,
        git: Result<&'static str, &'static str>,
    ) -> Option<String> {
        let mut outside = Scripted {
            github_email: github,
            git_email: git,
            ..Scripted::unattended()
        };
        setup(&mut outside).unwrap();
        let text = outside.user_config.clone().unwrap();
        let config = table(&outside);
        let to = config["email"]
            .get("to")
            .map(|to| to.as_str().unwrap().to_string());
        assert_eq!(
            text.contains("\n# to = \"you@example.com\""),
            to.is_none(),
            "{text}"
        );
        to
    }

    #[test]
    fn the_github_profiles_public_email_is_suggested_first() {
        let to = suggested(Ok(Some("octo@example.com")), Ok("me@example.org"));

        assert_eq!(to.as_deref(), Some("octo@example.com"));
    }

    #[test]
    fn with_no_public_github_email_or_none_readable_the_git_email_is_suggested() {
        for github in [Ok(None), Ok(Some("")), Err("gh: HTTP 401")] {
            let to = suggested(github, Ok("me@example.org"));

            assert_eq!(to.as_deref(), Some("me@example.org"), "{github:?}");
        }
    }

    #[test]
    fn a_noreply_git_email_is_never_suggested() {
        for noreply in [
            "123+octo@users.noreply.github.com",
            "123+octo@Users.NoReply.GitHub.com",
        ] {
            assert_eq!(suggested(Ok(None), Ok(noreply)), None, "{noreply}");
        }
    }

    #[test]
    fn with_neither_email_email_to_is_written_commented_out() {
        assert_eq!(suggested(Err("gh: HTTP 401"), Err("no user.email")), None);
        assert_eq!(suggested(Ok(None), Ok("")), None);
    }

    #[test]
    fn the_suggested_address_written_has_the_comment_of_the_line_it_replaces() {
        let mut outside = Scripted {
            github_email: Ok(Some("octo@example.com")),
            ..Scripted::unattended()
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            outside.user_config,
            Some(config::with_email_to(Some("octo@example.com".to_string())))
        );
    }

    #[test]
    fn setup_never_replaces_an_existing_email_to() {
        let mine = "[email]\nto = \"mine@example.net\"\n";
        let mut outside = Scripted {
            github_email: Ok(Some("octo@example.com")),
            user_config: Some(mine.to_string()),
            ..Scripted::unattended()
        };

        setup(&mut outside).unwrap();

        assert_eq!(
            table(&outside)["email"]["to"].as_str(),
            Some("mine@example.net")
        );
    }

    #[test]
    fn on_a_terminal_the_address_defaults_to_the_suggested_github_email() {
        let mut outside = Scripted {
            github_email: Ok(Some("octo@example.com")),
            ..Scripted::answering(&[
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (NOTIFY, "y"),
                (TO, ""),
                (FROM, ""),
                (KEY_PROMPT, ""),
            ])
        };

        setup(&mut outside).unwrap();

        assert!(
            outside
                .prompts()
                .contains(&"Send Run notifications to [octo@example.com]: "),
            "{:?}",
            outside.prompts()
        );
        let config = table(&outside);
        assert_eq!(config["email"]["always"].as_bool(), Some(true));
        assert_eq!(config["email"]["to"].as_str(), Some("octo@example.com"));
        assert_eq!(
            config["email"]["from"].as_str(),
            Some(crate::email::DEFAULT_FROM)
        );
    }

    // The first Run's offer.

    #[test]
    fn with_no_terminal_a_run_offers_nothing_and_writes_nothing() {
        let mut outside = Scripted::unattended();

        offer(&mut outside).unwrap();

        assert!(outside.calls.is_empty(), "{:?}", outside.calls);
    }

    #[test]
    fn with_a_user_config_a_run_offers_nothing() {
        let mut outside = Scripted {
            user_config: Some(String::new()),
            ..Scripted::answering(&[])
        };

        offer(&mut outside).unwrap();

        assert!(outside.calls.is_empty(), "{:?}", outside.calls);
    }

    #[test]
    fn declining_writes_the_defaults_and_asks_nothing_more() {
        let mut outside = Scripted {
            key: Err("unknown key resend.kye in the Credentials"),
            ..Scripted::answering(&[(OFFER, "n")])
        };

        offer(&mut outside).unwrap();

        assert_eq!(outside.prompts(), [OFFER]);
        assert_eq!(
            outside.effects(),
            [
                &Call::WriteNew(DEFAULTS.to_string()),
                &step(&format!(
                    "wrote the User config {PATH} with every setting at its default; \
                     thirdshift setup changes it"
                )),
            ]
        );
    }

    #[test]
    fn declining_writes_the_suggested_address() {
        let mut outside = Scripted {
            github_email: Ok(Some("octo@example.com")),
            ..Scripted::answering(&[(OFFER, "n")])
        };

        offer(&mut outside).unwrap();

        assert_eq!(
            outside.user_config,
            Some(config::with_email_to(Some("octo@example.com".to_string())))
        );
    }

    #[test]
    fn accepting_asks_the_setup_questions_and_writes_the_answers() {
        let mut outside = Scripted::answering(&[
            (OFFER, ""),
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, "y"),
            (BASE_FIX, ""),
            (PULL, ""),
            (NOTIFY, ""),
        ]);

        offer(&mut outside).unwrap();

        assert_eq!(
            outside.effects()[1..],
            [&step(&format!("wrote the User config {PATH}"))]
        );
        assert_eq!(table(&outside)["merge"]["always"].as_bool(), Some(true));
    }

    #[test]
    fn accepting_saves_the_key_after_the_user_config_and_sends_the_test_email_last() {
        let mut outside = Scripted::answering(&[
            (OFFER, "y"),
            (HARNESS, ""),
            (MODEL, ""),
            (EFFORT, ""),
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, ""),
            (KEY_PROMPT, KEY),
            (TEST_EMAIL, "y"),
        ]);

        offer(&mut outside).unwrap();

        assert_eq!(
            outside.effects()[1..],
            [
                &step(&format!("wrote the User config {PATH}")),
                &Call::SaveKey(KEY.to_string()),
                &step(&format!("wrote the Credentials {CREDENTIALS}")),
                &Call::SendTest {
                    to: Some("me@example.com".to_string()),
                    from: Some(crate::email::DEFAULT_FROM.to_string()),
                },
                &step(SENT),
            ]
        );
    }

    #[test]
    fn an_unwritable_user_config_is_a_warning_and_the_run_carries_on_with_the_defaults() {
        let mut outside = Scripted {
            failing: vec![Fails::WriteNew],
            ..Scripted::answering(&notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "y")]))
        };
        outside.answers.push_front((OFFER, "y"));

        offer(&mut outside).unwrap();

        assert_eq!(
            outside.effects()[1..],
            [&step(&format!(
                "warning: can't write {PATH}: Permission denied (os error 13); carrying on \
                 with the defaults"
            ))]
        );
        assert_eq!(outside.user_config, None);
    }

    #[test]
    fn credentials_that_cant_be_saved_or_a_test_email_that_cant_go_are_warnings() {
        let mut outside = Scripted {
            failing: vec![Fails::SaveKey, Fails::SendTest],
            ..Scripted::answering(&notifications_on(&[(KEY_PROMPT, KEY), (TEST_EMAIL, "y")]))
        };
        outside.answers.push_front((OFFER, "y"));

        offer(&mut outside).unwrap();

        let steps: Vec<&Call> = outside
            .calls
            .iter()
            .filter(|call| matches!(call, Call::Step(_)))
            .collect();
        assert_eq!(
            steps,
            [
                &step(&format!("wrote the User config {PATH}")),
                &step(&format!(
                    "warning: can't write {CREDENTIALS}: Permission denied (os error 13)"
                )),
                &step("warning: Permission denied (os error 13)"),
            ]
        );
    }

    #[test]
    fn accepting_with_broken_credentials_ends_the_command_before_any_question() {
        let mut outside = Scripted {
            key: Err("unknown key resend.kye in the Credentials"),
            ..Scripted::answering(&[(OFFER, "y")])
        };

        let error = offer(&mut outside).unwrap_err();

        assert_eq!(
            error.to_string(),
            "unknown key resend.kye in the Credentials"
        );
        assert_eq!(outside.prompts(), [OFFER]);
        assert!(!wrote_anything(&outside));
    }

    #[test]
    fn stdin_closing_at_the_offer_or_during_the_questions_writes_nothing() {
        for answers in [
            vec![],
            vec![(OFFER, "y"), (HARNESS, ""), (MODEL, ""), (EFFORT, "")],
            vec![
                (OFFER, "y"),
                (HARNESS, ""),
                (MODEL, ""),
                (EFFORT, ""),
                (MERGE, ""),
                (PULL, ""),
                (NOTIFY, "y"),
                (TO, "me@example.com"),
                (FROM, ""),
            ],
        ] {
            let mut outside = Scripted::answering(&answers);

            let error = offer(&mut outside).unwrap_err();

            assert_eq!(error.to_string(), ENDED, "{answers:?}");
            assert!(!wrote_anything(&outside), "{answers:?}");
        }
    }

    // Writing the answers into the text.

    fn notifications_to(to: &str) -> Answers {
        Answers {
            harness: None,
            merge_always: false,
            base_fix: false,
            launch_pull: false,
            notifications: Some(questions::Notifications {
                to: to.to_string(),
                from: crate::email::DEFAULT_FROM.to_string(),
                key: None,
                send_test: false,
            }),
        }
    }

    #[test]
    fn an_answered_address_takes_the_place_of_the_commented_out_line() {
        let answered = with_answers(DEFAULTS, &notifications_to("me@example.com")).unwrap();
        let expected = config::with_email_to(Some("me@example.com".to_string())).replace(
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
            &config::complete(text).unwrap(),
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
