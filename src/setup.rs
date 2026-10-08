//! Setup: writing the User config by answering a few questions, from
//! `thirdshift setup` or the first Run's offer of it, and the Credentials
//! when a Resend API key is entered.
//!
//! Its rules are here: whether to ask, the questions and their defaults, the
//! suggested address, what is written and in what order, and which failures
//! the offer carries on through. Preparing and editing a User config are
//! the User config module's. Everything Setup does outside
//! itself goes through [`Outside`]: [`OnMachine`] does each on the terminal,
//! through the complete Harness interaction, `gh`, `git`, Resend and the files
//! under the home folder; `Scripted`, in tests, from a script, recording each call.

pub(crate) mod questions;

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{self, EmailSettings, UserConfig, UserConfigDocument};
use crate::git::Git;
use crate::github::GitHub;
use crate::harness::{
    Harness, ModelAndEffort, Settings,
    settings::{self, Terminal},
};
use crate::resend_key::{Credentials, Source};
use crate::{email, interrupt, progress};

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
/// Run would refuse, stdin closing, or recorded interruption during questions
/// or checks end the command before any work, with nothing written.
pub fn offer() -> Result<()> {
    let (home, path) = config::home_and_path()?;
    run_offer(&mut OnMachine::new(&home), &home, &path)
}

/// [`setup`], for the User config at `path` under `home`.
fn run_setup(outside: &mut impl Outside, home: &Path, path: &Path) -> Result<String> {
    let existing = outside
        .read_user_config(path)
        .with_context(|| format!("can't read {}", path.display()))?;
    let document = match &existing {
        Some(text) => UserConfigDocument::existing(text, path, home)?,
        None => UserConfigDocument::defaults(path, home, suggested_address(outside))?,
    };
    let asking = outside.has_terminal();
    let (text, answered) = ask_if(asking, outside, document)?;
    let asked = answered.is_some();
    interrupt::check()?;
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
    let document = UserConfigDocument::defaults(path, home, suggested_address(outside))?;
    let (text, answered) = ask_if(accepted, outside, document)?;
    interrupt::check()?;
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

/// Ask using the prepared document's settings as question defaults, then
/// render the semantic answers. Credentials a Run would refuse are refused
/// before any Setup question. With no questions, render completed defaults.
fn ask_if(
    asking: bool,
    outside: &mut impl Outside,
    document: UserConfigDocument,
) -> Result<(String, Option<Answers>)> {
    let answers = if asking {
        let found = outside.find_key()?;
        Some(questions::ask(outside, document.settings(), found)?)
    } else {
        None
    };
    let changes = answers.as_ref().map(Answers::changes);
    Ok((document.render(changes), answers))
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

/// What Setup does outside itself: ordinary and hidden terminal input, the
/// complete Harness interaction, Credentials, suggested-address lookups, the
/// User config file, the test email, and where its progress lines go.
pub(crate) trait Outside: Terminal {
    /// Whether there is someone to ask: stdin and stderr are both terminals.
    fn has_terminal(&mut self) -> bool;
    /// Ordinary input, hidden for Credentials.
    fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>>;
    /// One complete settled Harness answer, retention, or an error.
    fn ask_harness_settings(
        &mut self,
        current: &Settings,
    ) -> Result<Option<(Harness, ModelAndEffort)>>;
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

impl Terminal for OnMachine<'_> {
    fn read(&mut self, prompt: &str) -> Result<Option<String>> {
        interrupt::install()?;
        interrupt::check()?;
        let mut stderr = std::io::stderr();
        let _ = write!(stderr, "{prompt}");
        let _ = stderr.flush();
        let line = terminal_line()?;
        if line.is_none() {
            let _ = writeln!(stderr);
        }
        Ok(line)
    }

    fn say(&mut self, line: String) {
        let _ = writeln!(std::io::stderr(), "{line}");
    }
}

impl Outside for OnMachine<'_> {
    fn has_terminal(&mut self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    /// With the terminal's echo off, so what is typed never shows.
    fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>> {
        interrupt::install()?;
        interrupt::check()?;
        let echo_off = EchoOff::new()?;
        let line = self.read(prompt)?;
        drop(echo_off);
        if line.is_some() {
            let _ = writeln!(std::io::stderr());
        }
        Ok(line)
    }

    fn ask_harness_settings(
        &mut self,
        current: &Settings,
    ) -> Result<Option<(Harness, ModelAndEffort)>> {
        settings::ask(self, current)
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
        GitHub::new().profile_email()
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

/// Read through the descriptor without stdin read-ahead, leaving pasted later
/// answers available to the next question. Polling lets recorded signals end
/// ordinary and hidden input even when there is no next line.
fn terminal_line() -> Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        interrupt::check()?;
        let mut stdin = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: stdin points to one valid pollfd; polling does not own fd 0.
        let ready = unsafe { libc::poll(&mut stdin, 1, 100) };
        interrupt::check()?;
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if ready == 0 {
            continue;
        }
        if stdin.revents & libc::POLLNVAL != 0 {
            return Err(std::io::Error::from_raw_os_error(libc::EBADF).into());
        }
        let mut byte = 0u8;
        // SAFETY: byte is writable for the one byte requested from fd 0.
        let read = unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) };
        interrupt::check()?;
        if read < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if read == 0 {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        line.push(byte);
        if byte == b'\n' {
            break;
        }
    }
    let line = String::from_utf8(line).context("terminal input is not UTF-8")?;
    interrupt::check()?;
    Ok(Some(line.trim().to_string()))
}

/// Own the exact saved terminal settings while echo is suppressed. Every
/// return path, including recorded interruption, restores them by dropping.
struct EchoOff {
    saved: libc::termios,
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
        let echo_off = EchoOff { saved };
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
    }
}

#[cfg(test)]
mod scripted {
    use std::collections::VecDeque;

    use anyhow::{anyhow, bail};

    use super::*;

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
        /// It requested one complete interaction with these current settings.
        HarnessSettings(Box<Settings>),
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
    /// answers; the complete Harness interaction outcome; where a key is found;
    /// the two emails; the User config, if any; and what fails.
    pub struct Scripted {
        pub terminal: bool,
        /// Each answer, with part of the prompt it is for. Once they run
        /// out, stdin is closed.
        pub answers: VecDeque<(&'static str, &'static str)>,
        pub harness: Result<Option<(Harness, ModelAndEffort)>, &'static str>,
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
        /// retaining Harness settings, with no key, no email to suggest and no
        /// User config.
        pub fn answering(answers: &[(&'static str, &'static str)]) -> Self {
            Scripted {
                terminal: true,
                answers: answers.iter().copied().collect(),
                harness: Ok(None),
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
                        Call::Ask(_) | Call::AskHidden(_) | Call::Say(_) | Call::HarnessSettings(_)
                    )
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

    impl Terminal for Scripted {
        fn read(&mut self, prompt: &str) -> Result<Option<String>> {
            self.calls.push(Call::Ask(prompt.to_string()));
            Ok(self.answer(prompt))
        }

        fn say(&mut self, line: String) {
            self.calls.push(Call::Say(line));
        }
    }

    impl Outside for Scripted {
        fn has_terminal(&mut self) -> bool {
            self.terminal
        }

        fn read_hidden(&mut self, prompt: &str) -> Result<Option<String>> {
            self.calls.push(Call::AskHidden(prompt.to_string()));
            Ok(self.answer(prompt))
        }

        fn ask_harness_settings(
            &mut self,
            current: &Settings,
        ) -> Result<Option<(Harness, ModelAndEffort)>> {
            self.calls
                .push(Call::HarnessSettings(Box::new(current.clone())));
            self.harness.clone().map_err(|error| anyhow!("{error}"))
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
    const ENTER_THROUGHOUT: [(&str, &str); 3] = [(MERGE, ""), (PULL, ""), (NOTIFY, "")];

    /// The answers that turn Run notifications on, to `me@example.com` from
    /// the default sender, then `rest`.
    fn notifications_on(
        rest: &[(&'static str, &'static str)],
    ) -> Vec<(&'static str, &'static str)> {
        let mut answers = vec![
            (MERGE, ""),
            (PULL, ""),
            (NOTIFY, "y"),
            (TO, "me@example.com"),
            (FROM, ""),
        ];
        answers.extend_from_slice(rest);
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

    // What Setup writes.

    #[test]
    fn the_harness_interaction_receives_every_current_setting_once_before_other_questions() {
        let mine = "[harness]\ndefault = 'codex'\n[harness.claude]\nmodel = 'opus'\neffort = 'high'\n[harness.codex]\nmodel = 'GPT-6-Luna'\neffort = 'High'\n[harness.agy]\nmodel = 'gemini-3.8-flash'\neffort = 'medium'\n[harness.grok]\nmodel = 'grok-4.5'\neffort = 'low'\n[harness.muse]\nmodel = 'muse-spark-1.3'\neffort = 'high'\n[harness.opencode]\nmodel = 'provider/model'\neffort = 'high'\n";
        for existing in [None, Some(mine)] {
            let mut outside = Scripted {
                user_config: existing.map(String::from),
                ..Scripted::answering(&ENTER_THROUGHOUT)
            };
            let expected = match existing {
                Some(text) => UserConfig::parse(text, Path::new(PATH), Path::new(HOME)).unwrap(),
                None => UserConfig::parse(DEFAULTS, Path::new(PATH), Path::new(HOME)).unwrap(),
            };

            setup(&mut outside).unwrap();

            assert_eq!(
                outside.calls[0],
                Call::HarnessSettings(Box::new(expected.harness))
            );
            assert!(matches!(&outside.calls[1], Call::Ask(prompt) if prompt.contains(MERGE)));
            assert_eq!(
                outside
                    .calls
                    .iter()
                    .filter(|call| matches!(call, Call::HarnessSettings(_)))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn an_accepted_harness_answer_persists_the_complete_choice_and_preserves_other_harnesses() {
        for harness in Harness::ALL {
            let mut outside = Scripted {
                user_config: Some(DEFAULTS.to_string()),
                harness: Ok(Some((
                    harness,
                    ModelAndEffort {
                        model: Some("settled-model".into()),
                        effort: Some("high".into()),
                    },
                ))),
                ..Scripted::answering(&ENTER_THROUGHOUT)
            };

            setup(&mut outside).unwrap();

            let config = table(&outside);
            assert_eq!(config["harness"]["default"].as_str(), Some(harness.name()));
            for other in Harness::ALL {
                let selected = other == harness;
                assert_eq!(
                    config["harness"][other.name()]["model"].as_str(),
                    Some(if selected { "settled-model" } else { "" })
                );
                assert_eq!(
                    config["harness"][other.name()]["effort"].as_str(),
                    Some(if selected { "high" } else { "" })
                );
            }
        }
    }

    #[test]
    fn retention_preserves_the_default_and_every_harness_setting() {
        let mine = DEFAULTS
            .replace("default = \"claude\"", "default = \"codex\"")
            .replace("model = \"\"", "model = \"my-model\"")
            .replace("effort = \"\"", "effort = \"high\"");
        let mut outside = Scripted {
            user_config: Some(mine.clone()),
            harness: Ok(None),
            ..Scripted::answering(&ENTER_THROUGHOUT)
        };

        setup(&mut outside).unwrap();

        assert_eq!(outside.user_config, Some(mine));
        assert!(!wrote_anything(&outside));
    }

    #[test]
    fn harness_interaction_errors_abort_setup_and_the_first_offer_before_any_write() {
        for error in [ENDED, "terminal read failed", "interrupted"] {
            for first_offer in [false, true] {
                let existing = (!first_offer).then(|| DEFAULTS.to_string());
                let mut outside = Scripted {
                    user_config: existing.clone(),
                    harness: Err(error),
                    ..Scripted::answering(if first_offer { &[(OFFER, "y")] } else { &[] })
                };

                let failed = if first_offer {
                    offer(&mut outside).map(|_| String::new())
                } else {
                    setup(&mut outside)
                };

                assert_eq!(failed.unwrap_err().to_string(), error);
                assert!(!wrote_anything(&outside));
                assert_eq!(outside.user_config, existing);
                assert_eq!(
                    outside.prompts(),
                    if first_offer { vec![OFFER] } else { vec![] }
                );
                assert!(matches!(
                    outside.calls.last(),
                    Some(Call::HarnessSettings(_))
                ));
            }
        }
    }

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
        let completed = UserConfigDocument::existing(partial, Path::new(PATH), Path::new(HOME))
            .unwrap()
            .render(None);
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
            ("merge = true\n", "merge must be the section [merge]"),
            (
                "[harness]\nclaude = true\n",
                "harness.claude must be the section [harness.claude]",
            ),
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

[security]
harness = \"codex\"

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
            ..Scripted::answering(&[(MERGE, ""), (PULL, "y"), (NOTIFY, "")])
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
            vec![(MERGE, "y"), (BASE_FIX, "")],
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
            let mut outside =
                Scripted::answering(&[(MERGE, "y"), (BASE_FIX, answer), (PULL, ""), (NOTIFY, "")]);

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
            let mut outside = Scripted::answering(&[(MERGE, answer), (PULL, ""), (NOTIFY, "")]);

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
            ..Scripted::answering(&[(MERGE, ""), (BASE_FIX, ""), (PULL, ""), (NOTIFY, "")])
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
            ..Scripted::answering(&[(MERGE, "n"), (PULL, ""), (NOTIFY, "")])
        };

        setup(&mut outside).unwrap();

        let text = outside.user_config.clone().unwrap();
        assert!(text.contains("fix = false # mine\n"), "{text}");
        assert_eq!(table(&outside)["merge"]["always"].as_bool(), Some(false));
    }

    #[test]
    fn a_yes_or_no_question_is_asked_again_until_the_answer_is_one() {
        let mut outside = Scripted::answering(&[
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
            ..Scripted::answering(&[(MERGE, ""), (PULL, ""), (NOTIFY, "n")])
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
            Some(
                UserConfigDocument::defaults(
                    Path::new(PATH),
                    Path::new(HOME),
                    Some("octo@example.com".to_string())
                )
                .unwrap()
                .render(None)
            )
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
            Some(
                UserConfigDocument::defaults(
                    Path::new(PATH),
                    Path::new(HOME),
                    Some("octo@example.com".to_string())
                )
                .unwrap()
                .render(None)
            )
        );
    }

    #[test]
    fn accepting_asks_the_setup_questions_and_writes_the_answers() {
        let mut outside = Scripted::answering(&[
            (OFFER, ""),
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
            vec![(OFFER, "y")],
            vec![
                (OFFER, "y"),
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
}
