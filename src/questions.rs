//! Setup's questions, asked on the terminal: on stderr, with the answers read
//! from stdin, so stdout stays empty. Each question's default answer, taken
//! by pressing Enter, is the current value. The Resend API key is read with
//! echo off, and never shown.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use signal_hook::SigId;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::low_level;

use crate::config::UserConfig;
use crate::email::DEFAULT_FROM;
use crate::resend_key::{self, Credentials, Source};

/// What the user chose.
pub struct Answers {
    /// `merge.always`.
    pub merge_always: bool,
    /// `launch.pull`.
    pub launch_pull: bool,
    /// With Run notifications on, `email.always`, their settings; with them
    /// off, `None`, and the email settings stay as they were.
    pub notifications: Option<Notifications>,
}

impl Answers {
    /// Whether the user asked for a test email.
    pub fn send_test(&self) -> bool {
        self.notifications
            .as_ref()
            .is_some_and(|notifications| notifications.send_test)
    }

    /// The Resend API key the user gave, if any.
    pub fn key(&self) -> Option<&str> {
        self.notifications
            .as_ref()
            .and_then(|notifications| notifications.key.as_deref())
    }
}

/// The settings of Run notifications, when the user wants them.
pub struct Notifications {
    /// `email.to`.
    pub to: String,
    /// `email.from`.
    pub from: String,
    /// A Resend API key to save in the Credentials, if the user gave one.
    pub key: Option<String>,
    /// Send a test email once the User config is written.
    pub send_test: bool,
}

/// Whether there is someone to ask: stdin and stderr are both terminals.
pub fn has_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// The first Run's offer of Setup, with no User config at `path`: whether
/// the user wants to set their defaults now, yes unless they say no.
pub fn offer(path: &Path) -> Result<bool> {
    yes_or_no(
        &format!(
            "No User config at {}. Set your defaults now?",
            path.display()
        ),
        true,
    )
}

/// Ask the Setup questions, with the settings in `current` as the default
/// answers, and `suggested_address` for `email.to` when `current` has none.
/// The address is re-asked until it has an `@`. With Run notifications on,
/// the Resend API key is asked for too, unless `RESEND_API_KEY` gives it,
/// with Enter keeping the one in `credentials`, if any. Ends in an error if
/// stdin closes before the last answer.
pub fn ask(
    current: &UserConfig,
    credentials: &Credentials,
    suggested_address: impl FnOnce() -> Option<String>,
) -> Result<Answers> {
    let merge_always = yes_or_no("Every Run a Merge run?", current.merge_always)?;
    let launch_pull = yes_or_no(
        "Every Run first fast-forwards your checkout of the Base branch?",
        current.launch_pull,
    )?;
    if !yes_or_no(
        "Run notifications, an email as each Run ends?",
        current.email.always,
    )? {
        return Ok(Answers {
            merge_always,
            launch_pull,
            notifications: None,
        });
    }
    let suggested = current.email.to.clone().or_else(suggested_address);
    let to = loop {
        let to = answer("Send Run notifications to", suggested.as_deref())?;
        if to.contains('@') {
            break to;
        }
    };
    let from = answer(
        "Send them from",
        Some(current.email.from.as_deref().unwrap_or(DEFAULT_FROM)),
    )?;
    let found = credentials.lookup().map(|key| key.source);
    let key = match found {
        Some(Source::Environment) => {
            say("The Resend API key comes from RESEND_API_KEY.");
            None
        }
        Some(Source::Credentials(_)) => ask_key("Enter keeps the saved one")?,
        None => ask_key("Enter to skip")?,
    };
    let send_test = if found.is_some() || key.is_some() {
        yes_or_no("Send a test email now?", false)?
    } else {
        say(
            "No Resend API key, so no email can go yet. To add one later, rerun \
             `thirdshift setup`, or set RESEND_API_KEY in the environment the Run starts from.",
        );
        false
    };
    Ok(Answers {
        merge_always,
        launch_pull,
        notifications: Some(Notifications {
            to,
            from,
            key,
            send_test,
        }),
    })
}

/// Ask `question` until the answer is yes, no or nothing, which is `default`.
fn yes_or_no(question: &str, default: bool) -> Result<bool> {
    let choices = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        match read(&format!("{question} {choices} "))?
            .to_ascii_lowercase()
            .as_str()
        {
            "" => return Ok(default),
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {}
        }
    }
}

/// Ask `question` for a word, showing `default`, which nothing takes, if
/// there is one. With no default, nothing is asked again.
fn answer(question: &str, default: Option<&str>) -> Result<String> {
    let prompt = match default {
        Some(default) => format!("{question} [{default}]: "),
        None => format!("{question}: "),
    };
    loop {
        match (read(&prompt)?, default) {
            (answer, _) if !answer.is_empty() => return Ok(answer),
            (_, Some(default)) => return Ok(default.to_string()),
            (_, None) => {}
        }
    }
}

/// Ask for a Resend API key, hidden, until the answer is one or is nothing.
/// `on_enter` says, in the prompt, what nothing does; it is `None`.
fn ask_key(on_enter: &str) -> Result<Option<String>> {
    loop {
        match read_hidden(&format!("Resend API key (input hidden, {on_enter}): "))? {
            key if key.is_empty() => return Ok(None),
            key if resend_key::is_key(&key) => return Ok(Some(key)),
            _ => say("That isn't a Resend API key, which starts with re_."),
        }
    }
}

/// Show `prompt` on stderr and read one line from stdin, trimmed, with the
/// terminal's echo off, so what is typed never shows.
fn read_hidden(prompt: &str) -> Result<String> {
    let echo_off = EchoOff::new()?;
    let line = read(prompt)?;
    drop(echo_off);
    let _ = writeln!(std::io::stderr());
    Ok(line)
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

/// Show `prompt` on stderr and read one line from stdin, trimmed.
fn read(prompt: &str) -> Result<String> {
    let mut stderr = std::io::stderr();
    let _ = write!(stderr, "{prompt}");
    let _ = stderr.flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        let _ = writeln!(stderr);
        bail!("Setup ended before its last answer; nothing written");
    }
    Ok(line.trim().to_string())
}

/// Show `text` on stderr.
fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}
