//! Antigravity CLI: launch, free Model catalog, Setup and stream protocol.
mod catalog;
pub(crate) mod stream;
pub use catalog::Catalog;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::{Choice, Harness, ModelAndEffort, Settings};
use crate::progress::{self, Stream};
use crate::setup::{Outside, questions::ask_setting};
use anyhow::Result;
use std::path::Path;

pub struct Agy;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Agy, &Agy);

impl Adapter for Agy {
    fn name(&self) -> &'static str {
        "agy"
    }
    fn product_name(&self) -> &'static str {
        "Antigravity CLI"
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
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.agy
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.agy
    }
    fn environment(&self) -> &'static [(&'static str, &'static str)] {
        &[("AGY_CLI_DISABLE_AUTO_UPDATE", "true")]
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let mut args: Vec<String> = [
            "-p",
            "--dangerously-skip-permissions",
            "--output-format",
            "stream-json",
        ]
        .map(String::from)
        .to_vec();
        if let Some(model) = &choice.model {
            args.extend(["--model".to_string(), model.clone()]);
        }
        if let Some(effort) = &choice.effort {
            args.extend(["--effort".to_string(), effort.clone()]);
        }
        if let Some(id) = resume {
            args.extend(["--conversation".to_string(), id.to_string()]);
        }
        args.push(self.skill_loading().prompt(prompt));
        Invocation { args, stdin: None }
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        if choice.model.is_none() && choice.effort.is_none() {
            return Ok(());
        }
        progress::step("checking the Model and Effort against agy models");
        let settled = Catalog::read()?.settle(choice.model.as_deref(), choice.effort.as_deref())?;
        choice.model = settled.model;
        choice.effort = settled.effort;
        Ok(())
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Outside,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        let catalog = match outside.agy_catalog() {
            Ok(catalog) => catalog,
            Err(error) => {
                outside.say(format!("{error:#}\nThe harness settings stay as they are; rerun `thirdshift setup` once agy models works."));
                return Ok(None);
            }
        };
        outside.say(format!(
            "{}'s Models: {}",
            self.product_name(),
            catalog.listed().join(", ")
        ));
        let model = loop {
            let model = ask_setting(outside, "Model", Harness::Agy, current.model.as_deref())?;
            match catalog.model_name(model.as_deref()) {
                Ok(model) => break model,
                Err(error) => outside.say(format!("{error:#}")),
            }
        };
        outside.say(format!(
            "Efforts {} supports: {}",
            model.as_deref().unwrap_or("agy"),
            catalog.efforts(model.as_deref()).join(", ")
        ));
        loop {
            let effort = ask_setting(outside, "Effort", Harness::Agy, current.effort.as_deref())?;
            match catalog.settle(model.as_deref(), effort.as_deref()) {
                Ok(settled) => return Ok(Some(settled)),
                Err(error) => outside.say(format!("{error:#}")),
            }
        }
    }
    fn link_instruction_fallback(&self, worktree: &Path) -> Result<()> {
        super::instructions::link_fallback(worktree, "GEMINI.md", &["AGENTS.md"])
    }
    fn stream(&self, _worktree: &Path) -> Box<dyn Stream> {
        Box::new(stream::AgyProgress::default())
    }
}
