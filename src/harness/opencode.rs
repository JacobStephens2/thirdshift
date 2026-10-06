//! OpenCode: private standalone sessions, stdin prompts and an authoritative export.
pub(super) mod export;
mod stream;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::interpretation::{Interpretation, Retained};
use super::settings::{Terminal, ask_setting};
use super::{Choice, Harness, ModelAndEffort, Settings};
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

pub struct OpenCode;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::OpenCode, &OpenCode);

impl Adapter for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn product_name(&self) -> &'static str {
        "OpenCode"
    }
    fn project_skills(&self) -> &'static str {
        ".agents/skills"
    }
    fn skill_loading(&self) -> SkillLoading {
        SkillLoading::Tool
    }
    fn stop_signals(&self) -> &'static [libc::c_int] {
        &[libc::SIGTERM]
    }
    fn environment(&self) -> &'static [(&'static str, &'static str)] {
        &[("OPENCODE_DISABLE_AUTOUPDATE", "1")]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.opencode
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.opencode
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let mut args = ["run", "--standalone", "--format", "json", "--auto"]
            .map(String::from)
            .to_vec();
        if let Some(model) = &choice.model {
            let model = if let Some(effort) = &choice.effort {
                format!("{}#{effort}", model.split('#').next().unwrap_or(model))
            } else {
                model.clone()
            };
            args.extend(["-m".into(), model]);
        }
        if let Some(id) = resume {
            args.extend(["-s".into(), id.into()]);
        }
        Invocation {
            args,
            stdin: Some(self.skill_loading().prompt(prompt)),
        }
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        check_model_and_effort(&ModelAndEffort {
            model: choice.model.clone(),
            effort: choice.effort.clone(),
        })
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Terminal,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        loop {
            let chosen = ModelAndEffort {
                model: ask_setting(
                    outside,
                    "Model",
                    Harness::OpenCode,
                    current.model.as_deref(),
                )?,
                effort: ask_setting(
                    outside,
                    "Effort",
                    Harness::OpenCode,
                    current.effort.as_deref(),
                )?,
            };
            match check_model_and_effort(&chosen) {
                Ok(()) => return Ok(Some(chosen)),
                Err(error) => {
                    crate::interrupt::check()?;
                    outside.say(format!("{error:#}"));
                }
            }
        }
    }
    fn interpretation(&self, worktree: &Path, prompt: &str) -> Interpretation {
        Interpretation::new(
            self.name(),
            Box::new(stream::OpenCodeProgress::for_prompt(prompt)),
            Retained::OpenCode(worktree.to_path_buf()),
        )
    }
    fn link_instruction_fallback(&self, worktree: &Path) -> Result<()> {
        super::instructions::link_fallback(worktree, "AGENTS.md", &[])
    }
}

/// Setup and Commands make the same minimal call: there is no free catalog
/// without the shared service, and an invalid route fails before a model turn.
pub fn check_model_and_effort(chosen: &ModelAndEffort) -> Result<()> {
    if chosen.effort.is_some() && chosen.model.is_none() {
        bail!("the OpenCode Effort needs a Model: set model <provider>/<model> too");
    }
    let choice = Choice {
        harness: Harness::OpenCode,
        model: chosen.model.clone(),
        effort: chosen.effort.clone(),
        chosen_by: super::ChosenBy::UserConfig,
    };
    crate::progress::step("checking OpenCode's Model and Effort with a standalone test call");
    let invocation = OpenCode.session(&choice, None, "Reply with OK.");
    let output = super::process::output(
        &OpenCode,
        Command::new(OpenCode.name()).args(invocation.args),
        invocation.stdin.as_deref(),
    )
    .context("could not run opencode to check the Model and Effort")?;
    OpenCode
        .interpretation(Path::new("."), "Reply with OK.")
        .check_output(&output)
        .context("opencode refused the Model or Effort in its test call")?;
    Ok(())
}

/// OpenCode errors appear as strings or typed objects in its event/export formats.
fn error_text(error: &serde_json::Value) -> Option<&str> {
    error.as_str().or_else(|| error["message"].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resume_uses_the_streamed_session_id_with_every_flag_and_the_prompt_on_stdin() {
        let choice = Choice {
            harness: Harness::OpenCode,
            model: Some("provider/model".into()),
            effort: Some("high".into()),
            chosen_by: super::super::ChosenBy::Command,
        };
        let prompt = "/thirdshift-implement issue-url";
        let mut stream =
            OpenCode.interpretation(Path::new("/missing-thirdshift-test-worktree"), prompt);
        stream.condense(r#"{"type":"step_start","sessionID":"ses_eedb9657fffec0RAa52IqAvyM1","part":{"type":"step-start"}}"#);
        let ended = crate::harness::interpretation_tests::finish(stream)
            .outcome
            .unwrap();
        let invocation = OpenCode.session(&choice, ended.session_id.as_deref(), prompt);
        assert_eq!(
            invocation.args,
            [
                "run",
                "--standalone",
                "--format",
                "json",
                "--auto",
                "-m",
                "provider/model#high",
                "-s",
                "ses_eedb9657fffec0RAa52IqAvyM1"
            ]
        );
        assert_eq!(
            invocation.stdin.as_deref(),
            Some("Load thirdshift-implement with your skill tool. issue-url")
        );
        assert_eq!(
            OpenCode.environment(),
            [("OPENCODE_DISABLE_AUTOUPDATE", "1")]
        );
        assert!(ended.killed.is_empty());
    }
}
