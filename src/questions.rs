//! Setup's questions, asked on the terminal: on stderr, with the answers read
//! from stdin, so stdout stays empty. Each question's default answer, taken
//! by pressing Enter, is the current value.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Result, bail};

use crate::config::UserConfig;
use crate::email::DEFAULT_FROM;

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
}

/// The settings of Run notifications, when the user wants them.
pub struct Notifications {
    /// `email.to`.
    pub to: String,
    /// `email.from`.
    pub from: String,
    /// Send a test email once the User config is written.
    pub send_test: bool,
}

/// Whether there is someone to ask: stdin and stderr are both terminals.
pub fn has_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Ask the Setup questions, with the settings in `current` as the default
/// answers, and `suggested_address` for `email.to` when `current` has none.
/// The address is re-asked until it has an `@`. Ends in an error if stdin
/// closes before the last answer.
pub fn ask(
    current: &UserConfig,
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
    let has_key = std::env::var("RESEND_API_KEY").is_ok_and(|key| !key.is_empty());
    let send_test = if has_key {
        yes_or_no("Send a test email now?", false)?
    } else {
        say(
            "RESEND_API_KEY is unset or empty, so no email can go yet. Add this line to your shell \
             profile, with your Resend API key:\n\n    export RESEND_API_KEY=re_...\n",
        );
        false
    };
    Ok(Answers {
        merge_always,
        launch_pull,
        notifications: Some(Notifications {
            to,
            from,
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
