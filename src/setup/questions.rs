//! Setup's questions, asked through [`Outside`]: on the terminal, on stderr,
//! with the answers read from stdin, so stdout stays empty. Each question's
//! default answer, taken by pressing Enter, is the current value. The Resend
//! API key is read hidden, and never shown.

use std::path::Path;

use anyhow::{Result, bail};

use super::{Outside, suggested_address};
use crate::config::UserConfig;
use crate::email::DEFAULT_FROM;
use crate::harness::{self, Harness, ModelAndEffort};
use crate::resend_key::{self, Source};

/// What the user chose.
pub struct Answers {
    /// `harness.default`, and the Model and Effort of the Harness it names,
    /// each none for the Harness's own default; `None` when sessions can run
    /// on no Harness here, and the harness settings stay as they were.
    pub harness: Option<(Harness, ModelAndEffort)>,
    /// `merge.always`.
    pub merge_always: bool,
    /// `base.fix`: asked only with `merge.always` on, and otherwise its
    /// default, false.
    pub base_fix: bool,
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

/// The first Run's offer of Setup, with no User config at `path`: whether
/// the user wants to set their defaults now, yes unless they say no.
pub fn offer(outside: &mut impl Outside, path: &Path) -> Result<bool> {
    yes_or_no(
        outside,
        &format!(
            "No User config at {}. Set your defaults now?",
            path.display()
        ),
        true,
    )
}

/// Ask the Setup questions, with the settings in `current` as the default
/// answers, and the suggested address for `email.to` when `current` has
/// none. The Harness, Model and Effort come first, as [`ask_harness`] asks
/// them. Base fixes are asked about only with every Run a Merge run;
/// otherwise the answer is `base.fix`'s default, no.
/// The address is re-asked until it has an `@`. With Run notifications on,
/// the Resend API key is asked for too, unless `found`, where the key was
/// found, is `RESEND_API_KEY`, with Enter keeping the one in the
/// Credentials, if any. Ends in an error if stdin closes before the last
/// answer.
pub fn ask(
    outside: &mut impl Outside,
    current: &UserConfig,
    found: Option<Source>,
) -> Result<Answers> {
    let harness = ask_harness(outside, &current.harness)?;
    let merge_always = yes_or_no(outside, "Every Run a Merge run?", current.merge_always)?;
    let base_fix = merge_always
        && yes_or_no(
            outside,
            "Every Run may start a Base fix when the Base branch's CI is red?",
            current.base_fix,
        )?;
    let launch_pull = yes_or_no(
        outside,
        "Every Run first fast-forwards your checkout of the Base branch?",
        current.launch_pull,
    )?;
    if !yes_or_no(
        outside,
        "Run notifications, an email as each Run ends?",
        current.email.always,
    )? {
        return Ok(Answers {
            harness,
            merge_always,
            base_fix,
            launch_pull,
            notifications: None,
        });
    }
    let suggested = match &current.email.to {
        Some(to) => Some(to.clone()),
        None => suggested_address(outside),
    };
    let to = loop {
        let to = answer(outside, "Send Run notifications to", suggested.as_deref())?;
        if to.contains('@') {
            break to;
        }
    };
    let from = answer(
        outside,
        "Send them from",
        Some(current.email.from.as_deref().unwrap_or(DEFAULT_FROM)),
    )?;
    let key = match &found {
        Some(Source::Environment) => {
            outside.say("The Resend API key comes from RESEND_API_KEY.".to_string());
            None
        }
        Some(Source::Credentials(_)) => ask_key(outside, "Enter keeps the saved one")?,
        None => ask_key(outside, "Enter to skip")?,
    };
    let send_test = if found.is_some() || key.is_some() {
        yes_or_no(outside, "Send a test email now?", false)?
    } else {
        outside.say(
            "No Resend API key, so no email can go yet. To add one later, rerun \
             `thirdshift setup`, or set RESEND_API_KEY in the environment the Run starts from."
                .to_string(),
        );
        false
    };
    Ok(Answers {
        harness,
        merge_always,
        base_fix,
        launch_pull,
        notifications: Some(Notifications {
            to,
            from,
            key,
            send_test,
        }),
    })
}

/// Ask which Harness every Run's sessions run on, listing each, marked where
/// it isn't installed, and refusing that one; then its Model and Effort,
/// with those `current` sets for it as the defaults, checked as a Run
/// checks them, through the chosen adapter. The Harness's
/// default is the `current` one if it's installed, else the first one
/// installed, so `claude` when it is. With no
/// Harness to choose, or a chosen Harness with an unreadable catalog, the answer
/// is `None`.
fn ask_harness(
    outside: &mut impl Outside,
    current: &harness::Settings,
) -> Result<Option<(Harness, ModelAndEffort)>> {
    let installed: Vec<Harness> = Harness::ALL
        .into_iter()
        .filter(|harness| outside.installed(*harness))
        .collect();
    let harnesses: Vec<String> = Harness::ALL
        .iter()
        .map(|harness| {
            if installed.contains(harness) {
                harness.name().to_string()
            } else {
                format!("{} (not installed)", harness.name())
            }
        })
        .collect();
    let harnesses = harnesses.join(" or ");
    let default = current
        .default
        .filter(|harness| installed.contains(harness))
        .or_else(|| installed.first().copied());
    let Some(default) = default else {
        let names = harness::names();
        outside.say(format!(
            "Harness for every Run's sessions: {harnesses}. Sessions can run on none here, so \
             the harness settings stay as they are; install {names}, then rerun \
             `thirdshift setup`."
        ));
        return Ok(None);
    };
    let harness = loop {
        let name = answer(
            outside,
            &format!("Harness for every Run's sessions, {harnesses}"),
            Some(default.name()),
        )?;
        match Harness::named(&name) {
            None => outside.say(format!("Choose {}.", harness::names())),
            Some(harness) if !installed.contains(&harness) => {
                outside.say(format!("{name} is not installed: it isn't on PATH."))
            }
            Some(harness) => break harness,
        }
    };
    let adapter = harness.adapter();
    let chosen = adapter.ask_settings(outside, adapter.settings(current))?;
    Ok(chosen.map(|chosen| (harness, chosen)))
}

/// Ask for the `setting`, the Model or the Effort, of `harness`, with
/// `current` as the default; `-` is none, the Harness's own default.
pub(crate) fn ask_setting(
    outside: &mut (impl Outside + ?Sized),
    setting: &str,
    harness: Harness,
    current: Option<&str>,
) -> Result<Option<String>> {
    let name = harness.name();
    let own = format!("{name}'s own default");
    let answer = match current {
        Some(current) => answer(
            outside,
            &format!("{setting} for {name}, - for {own}"),
            Some(current),
        )?,
        None => read(outside, &format!("{setting} for {name} [{own}]: "))?,
    };
    Ok(Some(answer).filter(|answer| !answer.is_empty() && answer != "-"))
}

/// Ask one catalog-backed setting, retrying refusals with the catalog's
/// valid choices and returning the name the Harness takes.
pub(crate) fn ask_checked_setting(
    outside: &mut (impl Outside + ?Sized),
    setting: &str,
    harness: Harness,
    current: Option<&str>,
    settle: impl Fn(Option<&str>) -> Result<Option<String>>,
) -> Result<Option<String>> {
    loop {
        let answer = ask_setting(outside, setting, harness, current)?;
        match settle(answer.as_deref()) {
            Ok(settled) => return Ok(settled),
            Err(error) => outside.say(format!("{error:#}")),
        }
    }
}

/// Ask `question` until the answer is yes, no or nothing, which is `default`.
fn yes_or_no(outside: &mut impl Outside, question: &str, default: bool) -> Result<bool> {
    let choices = if default { "[Y/n]" } else { "[y/N]" };
    loop {
        match read(outside, &format!("{question} {choices} "))?
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
fn answer(
    outside: &mut (impl Outside + ?Sized),
    question: &str,
    default: Option<&str>,
) -> Result<String> {
    let prompt = match default {
        Some(default) => format!("{question} [{default}]: "),
        None => format!("{question}: "),
    };
    loop {
        match (read(outside, &prompt)?, default) {
            (answer, _) if !answer.is_empty() => return Ok(answer),
            (_, Some(default)) => return Ok(default.to_string()),
            (_, None) => {}
        }
    }
}

/// Ask for a Resend API key, hidden, until the answer is one or is nothing.
/// `on_enter` says, in the prompt, what nothing does; it is `None`.
fn ask_key(outside: &mut impl Outside, on_enter: &str) -> Result<Option<String>> {
    let prompt = format!("Resend API key (input hidden, {on_enter}): ");
    loop {
        match outside.read_hidden(&prompt)? {
            None => bail!(ENDED),
            Some(key) if key.is_empty() => return Ok(None),
            Some(key) if resend_key::is_key(&key) => return Ok(Some(key)),
            Some(_) => {
                outside.say("That isn't a Resend API key, which starts with re_.".to_string())
            }
        }
    }
}

/// Show `prompt` and read one answer, trimmed, or end Setup if stdin closes.
fn read(outside: &mut (impl Outside + ?Sized), prompt: &str) -> Result<String> {
    match outside.read(prompt)? {
        Some(line) => Ok(line),
        None => bail!(ENDED),
    }
}

/// Why Setup ends when stdin closes before the last answer.
const ENDED: &str = "Setup ended before its last answer; nothing written";
