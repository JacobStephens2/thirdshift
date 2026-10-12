//! Antigravity CLI: launch, free Model catalog, Setup and stream protocol.
mod catalog;
pub(crate) mod stream;
pub use catalog::Catalog;

use super::adapter::{Adapter, Invocation, SkillLoading, without};
use super::interpretation::Interpretation;
use super::settings::{Terminal, ask_setting};
use super::{Choice, Harness, ModelAndEffort, Settings};
use crate::progress;
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
        let mut args = vec!["-p".to_string(), self.skill_loading().prompt(prompt)];
        args.extend(
            [
                "--dangerously-skip-permissions",
                "--output-format",
                "stream-json",
            ]
            .map(String::from),
        );
        if let Some(model) = &choice.model {
            args.extend(["--model".to_string(), model.clone()]);
        }
        if let Some(effort) = &choice.effort {
            args.extend(["--effort".to_string(), effort.clone()]);
        }
        if let Some(id) = resume {
            args.extend(["--conversation".to_string(), id.to_string()]);
        }
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
    /// Headless agy auto-denies every tool that needs a permission: commands,
    /// writes, URLs and MCP, and a denial ends the turn with no summary. Its
    /// file reads need none, and no flag removes them, so a summary can
    /// still quote a file the model chose to read.
    fn summary(&self, choice: &Choice, prompt: &str, _prompt_file: &Path) -> Invocation {
        let mut invocation = self.session(choice, None, prompt);
        invocation.args.remove(1);
        invocation.args = without(invocation.args, &["--dangerously-skip-permissions"]);
        invocation.args[0] = "-p=".into();
        invocation.args.extend(
            [
                "--sandbox",
                "--disable-slash-commands",
                // A stalled turn fails, so the release falls back to
                // generated notes rather than waiting forever.
                "--print-timeout",
                "10m",
                "--input-format",
                "stream-json",
            ]
            .map(String::from),
        );
        invocation.stdin = Some(format!(
            "{}\n",
            serde_json::json!({
                "event": "user", "message": {"role": "user", "content": prompt}
            })
        ));
        invocation
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
    fn interpretation(&self, _worktree: &Path, _prompt: &str) -> Interpretation {
        Interpretation::new(self.name(), Box::new(stream::AgyProgress::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ChosenBy;

    /// Resume uses the same public invocation interface as a new session.
    #[test]
    fn a_resume_reuses_every_flag_and_environment_and_the_conversation_id() {
        let chosen = Choice {
            harness: Harness::Agy,
            model: Some("gemini-3.8-flash".to_string()),
            effort: Some("medium".to_string()),
            chosen_by: ChosenBy::Command,
        };
        let adapter = chosen.harness.adapter();
        let resume = adapter.session(
            &chosen,
            Some("recorded-conversation"),
            "/thirdshift-implement Continue.",
        );
        assert_eq!(
            resume.args,
            [
                "-p",
                "/thirdshift-implement Continue.",
                "--dangerously-skip-permissions",
                "--output-format",
                "stream-json",
                "--model",
                "gemini-3.8-flash",
                "--effort",
                "medium",
                "--conversation",
                "recorded-conversation"
            ]
        );
        assert_eq!(resume.stdin, None);
        assert_eq!(
            adapter.environment(),
            [("AGY_CLI_DISABLE_AUTO_UPDATE", "true")]
        );
    }

    #[test]
    #[ignore = "requires an installed, authenticated agy and makes two live Model calls"]
    fn real_agy_receives_new_and_resumed_prompts() {
        let directory = tempfile::tempdir().unwrap();
        let chosen = Choice {
            harness: Harness::Agy,
            model: Some("gemini-3.8-flash-low".into()),
            effort: None,
            chosen_by: ChosenBy::Command,
        };
        let adapter = chosen.harness.adapter();
        let mut conversation = None;
        for marker in ["AGY_NEW_658", "AGY_RESUMED_658"] {
            let prompt = format!("Do not use tools. Reply with exactly {marker}");
            let mut invocation = adapter.session(&chosen, conversation.as_deref(), &prompt);
            invocation
                .args
                .extend(["--print-timeout".into(), "2m".into()]);
            let output = crate::harness::process::output(
                adapter,
                std::process::Command::new(adapter.name())
                    .args(invocation.args)
                    .current_dir(directory.path()),
                invocation.stdin.as_deref(),
            )
            .unwrap();
            let mut interpretation = adapter.interpretation(directory.path(), &prompt);
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                interpretation.condense(line);
            }
            let ended = interpretation.finish(Ok(output.status)).outcome.unwrap();
            assert_eq!(ended.final_message.as_deref().map(str::trim), Some(marker));
            let id = ended
                .session_id
                .expect("agy must provide a conversation ID");
            if let Some(previous) = &conversation {
                assert_eq!(&id, previous, "Resume must keep the conversation ID");
            }
            println!("{marker}: SUCCESS (conversation {id})");
            conversation = Some(id);
        }
    }
}
