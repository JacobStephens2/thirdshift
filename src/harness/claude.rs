//! Claude Code: its launch protocol, Setup and Model check.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::interpretation::{Interpretation, Retained, Stream};
use super::settings::{Terminal, ask_setting};
use super::{Choice, Harness, ModelAndEffort, Settings, said};
use crate::progress;

pub struct Claude;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Claude, &Claude);

impl Adapter for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }
    fn product_name(&self) -> &'static str {
        "Claude Code"
    }
    fn project_skills(&self) -> &'static str {
        ".claude/skills"
    }
    fn skill_loading(&self) -> SkillLoading {
        SkillLoading::Slash
    }
    fn stop_signals(&self) -> &'static [libc::c_int] {
        &[libc::SIGTERM]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.claude
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.claude
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let prompt = self.skill_loading().prompt(prompt);
        Invocation {
            args: claude_args(choice, resume, &prompt)
                .into_iter()
                .map(String::from)
                .collect(),
            stdin: None,
        }
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        test_call(choice)
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Terminal,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        ask_claude(outside, current).map(Some)
    }
    fn interpretation(&self, _worktree: &Path, _prompt: &str) -> Interpretation {
        Interpretation::new(self.name(), Box::new(Stream::claude()), Retained::None)
    }
}

/// The arguments that run a Claude session on its Model and Effort,
/// where they are set.
pub fn model_args(choice: &Choice) -> Vec<&str> {
    let mut args = Vec::new();
    if let Some(model) = &choice.model {
        args.extend(["--model", model]);
    }
    if let Some(effort) = &choice.effort {
        args.extend(["--effort", effort]);
    }
    args
}

/// The arguments every session runs `claude` with: headless in auto mode,
/// on the Model and Effort `harness` sets, if any, streaming JSON. With
/// `resume`, the session with that id continues. The prompt comes last.
pub fn claude_args<'a>(
    harness: &'a Choice,
    resume: Option<&'a str>,
    prompt: &'a str,
) -> Vec<&'a str> {
    let mut args = vec!["-p", "--permission-mode", "auto"];
    args.extend(model_args(harness));
    args.extend(["--output-format", "stream-json", "--verbose"]);
    if let Some(session_id) = resume {
        args.extend(["--resume", session_id]);
    }
    args.push(prompt);
    args
}

/// Check that Claude takes a minimal test call on the Model, if one is
/// named, with the Effort, if any. A refusal says what Claude said.
pub fn test_call(choice: &Choice) -> Result<()> {
    let cli = Claude.name();
    let Some(model) = &choice.model else {
        return Ok(());
    };
    progress::step(format_args!(
        "checking the Model {model} with a test call to {cli}"
    ));
    let output = super::process::output(
        &Claude,
        Command::new(cli).arg("-p").args(model_args(choice)),
        Some(TEST_PROMPT),
    )
    .with_context(|| format!("could not run {cli} to check the Model {model}"))?;
    if output.status.success() {
        return Ok(());
    }
    let effort = match &choice.effort {
        Some(effort) => format!(" with the Effort {effort}"),
        None => String::new(),
    };
    bail!(
        "{cli} refused a test call on the Model {model}{effort}: {}",
        said(&output)
    )
}

/// What the minimal Model check asks.
const TEST_PROMPT: &str = "Reply with OK.";

/// Ask Claude's Model and Effort, with `current` as the defaults. A Model is
/// checked with a test call, as a Run checks it, and on a refusal the Model
/// and Effort are asked again.
fn ask_claude(outside: &mut dyn Terminal, current: &ModelAndEffort) -> Result<ModelAndEffort> {
    loop {
        let chosen = ModelAndEffort {
            model: ask_setting(outside, "Model", Harness::Claude, current.model.as_deref())?,
            effort: ask_setting(
                outside,
                "Effort",
                Harness::Claude,
                current.effort.as_deref(),
            )?,
        };
        if chosen.model.is_none() {
            return Ok(chosen);
        }
        match test_call(&Choice {
            harness: Harness::Claude,
            model: chosen.model.clone(),
            effort: chosen.effort.clone(),
            chosen_by: super::ChosenBy::UserConfig,
        }) {
            Ok(()) => return Ok(chosen),
            Err(error) => {
                crate::interrupt::check()?;
                outside.say(format!("{error:#}"));
            }
        }
    }
}
