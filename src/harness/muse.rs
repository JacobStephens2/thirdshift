//! Muse Code: unattended sessions, skill-tool loading and the read after exit.

mod catalog;
mod log;
mod stream;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::{Choice, Harness, ModelAndEffort, Settings, said};
use crate::progress::Stream;
use crate::setup::{Outside, questions::ask_setting};

pub struct Muse;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Muse, &Muse);

impl Adapter for Muse {
    fn name(&self) -> &'static str {
        "muse"
    }
    fn product_name(&self) -> &'static str {
        "Muse Code"
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
        &[("MUSE_NO_AUTO_UPDATE", "1")]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.muse
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.muse
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let mut args = ["exec", "--json", "--yolo"].map(String::from).to_vec();
        if let Some(model) = &choice.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(effort) = &choice.effort {
            args.extend(["--reasoning-effort".into(), effort.clone()]);
        }
        if let Some(id) = resume {
            args.extend(["--session-id".into(), id.into()]);
        }
        args.push(self.skill_loading().prompt(prompt));
        Invocation { args, stdin: None }
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        let settled = check_model_and_effort(&ModelAndEffort {
            model: choice.model.clone(),
            effort: choice.effort.clone(),
        })?;
        choice.model = settled.model;
        choice.effort = settled.effort;
        Ok(())
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Outside,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        loop {
            let chosen = ModelAndEffort {
                model: ask_setting(
                    outside,
                    "Model",
                    Harness::Muse,
                    current.model.as_deref().or(Some("muse-spark-1.3")),
                )?,
                effort: ask_setting(outside, "Effort", Harness::Muse, current.effort.as_deref())?,
            };
            let chosen = match catalog::settle_effort(chosen.effort.as_deref()) {
                Ok(effort) => ModelAndEffort { effort, ..chosen },
                Err(error) => {
                    outside.say(format!("{error:#}"));
                    continue;
                }
            };
            match outside.muse_check(&chosen) {
                Ok(settled) => return Ok(Some(settled)),
                Err(error) => outside.say(format!("{error:#}")),
            }
        }
    }
    fn stream(&self, _worktree: &Path, prompt: &str) -> Box<dyn Stream> {
        Box::new(stream::MuseProgress::for_prompt(prompt))
    }
    fn read_after_exit(&self, _worktree: &Path, stream: Box<dyn Stream>) -> Box<dyn Stream> {
        log::read_after_exit(stream)
    }
}

/// Muse's per-user data, where its catalog and session logs live.
fn data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .map(|dir| dir.join("muse"))
}

/// The same check used by a Command and Setup: validate Effort first, then
/// use the cached catalog, falling back to a minimal call only without it.
pub fn check_model_and_effort(chosen: &ModelAndEffort) -> Result<ModelAndEffort> {
    let mut settled = ModelAndEffort {
        model: chosen.model.clone(),
        effort: catalog::settle_effort(chosen.effort.as_deref())?,
    };
    if let Some(model) = &settled.model {
        if let Some(catalog) = catalog::Catalog::read()? {
            crate::progress::step("checking the Model against Muse's cached catalog");
            settled.model = Some(catalog.settle_model(model)?);
        } else {
            let choice = Choice {
                harness: Harness::Muse,
                model: settled.model.clone(),
                effort: settled.effort.clone(),
                chosen_by: super::ChosenBy::UserConfig,
            };
            crate::progress::step(format_args!(
                "checking the Model {model} with a test call to muse"
            ));
            let invocation = Muse.session(&choice, None, "Reply with OK.");
            let output = Command::new(Muse.name())
                .args(invocation.args)
                .envs(Muse.environment().iter().copied())
                .stdin(Stdio::null())
                .output()
                .context("could not run muse to check the Model")?;
            let mut stream = Muse.stream(Path::new("."), "Reply with OK.");
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                stream.condense(line);
            }
            if !output.status.success() || stream.failed() {
                bail!(
                    "muse refused a test call on the Model {model}: {}",
                    stream
                        .error()
                        .map(String::from)
                        .unwrap_or_else(|| said(&output))
                );
            }
        }
    }
    Ok(settled)
}
