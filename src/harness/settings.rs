//! Installed-Harness selection and its complete Model and Effort interaction.
//! A settled answer can be applied atomically; retention changes nothing.

use anyhow::{Result, bail};

use super::{Harness, ModelAndEffort, Settings};

/// Ordinary terminal input and output. Answers are trimmed; EOF is `None`.
pub trait Terminal {
    fn read(&mut self, prompt: &str) -> Result<Option<String>>;
    fn say(&mut self, line: String);
}

/// Ask which installed Harness every Run's sessions run on, then its
/// Model and Effort through the chosen adapter. Keep the configured default
/// when installed, otherwise prefer Claude, then the first installed.
/// With no Harness installed, leave the Harness settings as they are.
pub fn ask(
    outside: &mut impl Terminal,
    current: &Settings,
) -> Result<Option<(Harness, ModelAndEffort)>> {
    crate::interrupt::check()?;
    let installed: Vec<Harness> = Harness::ALL
        .into_iter()
        .filter(|harness| harness.installed())
        .collect();
    let harnesses = installed
        .iter()
        .map(|harness| harness.name())
        .collect::<Vec<_>>()
        .join(" or ");
    let default = current
        .default
        .filter(|harness| installed.contains(harness))
        .or_else(|| installed.first().copied());
    let Some(default) = default else {
        let names = super::names();
        outside.say(format!(
            "No Harness is installed here, so the harness settings stay as they are; \
             install {names}, then rerun `thirdshift setup`."
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
            None => outside.say(format!("Choose {harnesses}.")),
            Some(harness) if !installed.contains(&harness) => {
                outside.say(format!("{name} is not installed: it isn't on PATH."))
            }
            Some(harness) => break harness,
        }
    };
    let adapter = harness.adapter();
    let chosen = adapter.ask_settings(outside, adapter.settings(current))?;
    crate::interrupt::check()?;
    Ok(chosen.map(|chosen| (harness, chosen)))
}

/// Ask for the `setting`, the Model or the Effort, of `harness`, with
/// `current` as the default; `-` is none, the Harness's own default.
pub(crate) fn ask_setting(
    outside: &mut (impl Terminal + ?Sized),
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
    outside: &mut (impl Terminal + ?Sized),
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

/// Ask `question` for a word, showing `default`, which nothing takes, if
/// there is one. With no default, nothing is asked again.
fn answer(
    outside: &mut (impl Terminal + ?Sized),
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

/// Show `prompt` and read one answer, trimmed, or end Setup if stdin closes.
fn read(outside: &mut (impl Terminal + ?Sized), prompt: &str) -> Result<String> {
    crate::interrupt::check()?;
    let line = outside.read(prompt)?;
    crate::interrupt::check()?;
    match line {
        Some(line) => Ok(line),
        None => bail!(ENDED),
    }
}

/// Why Setup ends when stdin closes before the last answer.
const ENDED: &str = "Setup ended before its last answer; nothing written";

#[cfg(test)]
mod tests;
