//! Grok Build: unattended sessions and its Claude-shaped Messages stream.

mod catalog;
mod stream;
pub use catalog::Catalog;

use std::path::Path;

use anyhow::Result;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::{Choice, Harness, ModelAndEffort, Settings};
use crate::progress::Stream;
use crate::setup::{Outside, questions::ask_setting};

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
            "-p",
            "--always-approve",
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
        args.push(self.skill_loading().prompt(prompt));
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
        outside: &mut dyn Outside,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        let catalog = match outside.grok_catalog() {
            Ok(catalog) => catalog,
            Err(error) => {
                outside.say(format!("{error:#}\nThe harness settings stay as they are; rerun `thirdshift setup` once grok models works."));
                return Ok(None);
            }
        };
        outside.say(format!(
            "{}'s Models: {}",
            self.product_name(),
            catalog.listed().join(", ")
        ));
        let model = loop {
            let model = ask_setting(outside, "Model", Harness::Grok, current.model.as_deref())?;
            match catalog.settle(model.as_deref(), None) {
                Ok(settled) => break settled.model,
                Err(error) => outside.say(format!("{error:#}")),
            }
        };
        outside.say(format!(
            "Efforts {} supports: {}",
            model.as_deref().unwrap_or("Grok Build's default Model"),
            catalog.efforts(model.as_deref()).join(", ")
        ));
        loop {
            let effort = ask_setting(outside, "Effort", Harness::Grok, current.effort.as_deref())?;
            match catalog.settle(model.as_deref(), effort.as_deref()) {
                Ok(settled) => return Ok(Some(settled)),
                Err(error) => outside.say(format!("{error:#}")),
            }
        }
    }
    fn stream(&self, _worktree: &Path) -> Box<dyn Stream> {
        Box::new(stream::GrokProgress::default())
    }
}
