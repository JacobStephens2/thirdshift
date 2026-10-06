//! Grok Build: unattended sessions and its Claude-shaped Messages stream.

mod catalog;
mod stream;
pub use catalog::Catalog;

use std::path::Path;

use anyhow::Result;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::settings::{Terminal, ask_checked_setting};
use super::{Choice, Harness, ModelAndEffort, Settings};
use crate::progress::Stream;

pub struct Grok;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Grok, &Grok);

impl Adapter for Grok {
    fn name(&self) -> &'static str {
        "grok"
    }
    fn product_name(&self) -> &'static str {
        "Grok Build"
    }
    fn project_skills(&self) -> &'static str {
        ".agents/skills"
    }
    fn skill_loading(&self) -> SkillLoading {
        SkillLoading::Slash
    }
    fn stop_signals(&self) -> &'static [libc::c_int] {
        &[libc::SIGTERM]
    }
    fn environment(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("GROK_DISABLE_AUTOUPDATER", "1"),
            ("GROK_FOLDER_TRUST", "0"),
        ]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.grok
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.grok
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let mut args: Vec<String> = [
            "--always-approve",
            "--sandbox",
            "off",
            "--output-format",
            "streaming-messages-json",
        ]
        .map(String::from)
        .to_vec();
        if let Some(model) = &choice.model {
            args.extend(["-m".to_string(), model.clone()]);
        }
        if let Some(effort) = &choice.effort {
            args.extend(["--reasoning-effort".to_string(), effort.clone()]);
        }
        if let Some(id) = resume {
            args.extend(["-r".to_string(), id.to_string()]);
        }
        args.extend(["-p".to_string(), self.skill_loading().prompt(prompt)]);
        Invocation { args, stdin: None }
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        crate::progress::step("checking the Model and Effort against grok models");
        let settled = Catalog::read()?.settle(choice.model.as_deref(), choice.effort.as_deref())?;
        choice.model = settled.model;
        choice.effort = settled.effort;
        Ok(())
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Terminal,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        let catalog = match Catalog::read() {
            Ok(catalog) => catalog,
            Err(error) => {
                crate::interrupt::check()?;
                outside.say(format!("{error:#}\nThe harness settings stay as they are; rerun `thirdshift setup` once grok models works."));
                return Ok(None);
            }
        };
        outside.say(format!(
            "{}'s Models: {}",
            self.product_name(),
            catalog.listed().join(", ")
        ));
        let model = ask_checked_setting(
            outside,
            "Model",
            Harness::Grok,
            current.model.as_deref(),
            |model| catalog.settle(model, None).map(|settled| settled.model),
        )?;
        outside.say(format!(
            "Efforts {} supports: {}",
            model.as_deref().unwrap_or("Grok Build's default Model"),
            catalog.efforts(model.as_deref()).join(", ")
        ));
        let effort = ask_checked_setting(
            outside,
            "Effort",
            Harness::Grok,
            current.effort.as_deref(),
            |effort| {
                catalog
                    .settle(model.as_deref(), effort)
                    .map(|settled| settled.effort)
            },
        )?;
        Ok(Some(ModelAndEffort { model, effort }))
    }
    fn stream(&self, _worktree: &Path, _prompt: &str) -> Box<dyn Stream> {
        Box::new(stream::GrokProgress::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ChosenBy;

    #[test]
    fn a_resume_uses_a_streamed_id_with_the_same_flags_null_stdin_and_fixed_environment() {
        let mut progress = stream::GrokProgress::default();
        progress
            .condense(r#"{"type":"system","subtype":"init","session_id":"abc123","cwd":"/repo"}"#);
        let choice = Choice {
            harness: Harness::Grok,
            model: Some("grok-4.7".to_string()),
            effort: Some("low".to_string()),
            chosen_by: ChosenBy::Command,
        };
        let adapter = choice.harness.adapter();
        let resume = adapter.session(&choice, progress.session_id(), "Continue.");
        assert_eq!(
            resume.args,
            [
                "--always-approve",
                "--sandbox",
                "off",
                "--output-format",
                "streaming-messages-json",
                "-m",
                "grok-4.7",
                "--reasoning-effort",
                "low",
                "-r",
                "abc123",
                "-p",
                "Continue."
            ]
        );
        assert_eq!(resume.stdin, None);
        assert_eq!(
            adapter.environment(),
            [
                ("GROK_DISABLE_AUTOUPDATER", "1"),
                ("GROK_FOLDER_TRUST", "0")
            ]
        );
    }
}
