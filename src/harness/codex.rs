//! Codex: its launch protocol, free Model catalog and Setup.

mod catalog;
pub(crate) mod stream;
pub use catalog::Catalog;

use std::path::Path;

use anyhow::Result;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::interpretation::{Interpretation, Retained};
use super::settings::{Terminal, ask_checked_setting};
use super::{Choice, Harness, ModelAndEffort, Settings};
use crate::progress;

pub struct Codex;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Codex, &Codex);

impl Adapter for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }
    fn product_name(&self) -> &'static str {
        "Codex"
    }
    fn project_skills(&self) -> &'static str {
        ".agents/skills"
    }
    fn skill_loading(&self) -> SkillLoading {
        SkillLoading::Dollar
    }
    fn stop_signals(&self) -> &'static [libc::c_int] {
        &[libc::SIGINT, libc::SIGTERM]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.codex
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.codex
    }
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        Invocation {
            args: codex_args(choice, resume, &self.skill_loading().prompt(prompt)),
            stdin: None,
        }
    }
    fn security_session(
        &self,
        choice: &Choice,
        resume: Option<&str>,
        prompt: &str,
        fresh_sub_agents: bool,
    ) -> Invocation {
        let prompt = if fresh_sub_agents {
            format!(
                "{prompt}\nStart fresh sub-agents with `fork_turns: \"none\"`, giving each only its own task and necessary evidence, so the skill's verifiers stay independent.\n"
            )
        } else {
            prompt.to_string()
        };
        let mut invocation = self.session(choice, resume, &prompt);
        // After `exec`, before any Resume subcommand: repeated on each launch.
        invocation.args.splice(
            1..1,
            [
                "-c".to_string(),
                "agents.max_concurrent_threads_per_session=8".to_string(),
            ],
        );
        invocation
    }
    fn check(&self, choice: &mut Choice) -> Result<()> {
        check_model_and_effort(choice)
    }
    /// Unlike a session (ADR-0012), the summary reads contributor-editable PR
    /// titles and bodies, so it runs sandboxed and read-only, with nothing to
    /// approve and Codex's shell, MCP, browser and hook tools switched off,
    /// as Claude's summary runs with no tools. A Codex that doesn't know one
    /// of these features fails, and the release falls back to generated notes.
    fn summary(&self, choice: &Choice, prompt: &str, _prompt_file: &Path) -> Invocation {
        let mut args: Vec<String> = [
            "exec",
            // Summary generation runs in a temporary directory, outside Git.
            "--skip-git-repo-check",
            "--json",
            "--ephemeral",
            "--sandbox",
            "read-only",
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "web_search=\"disabled\"",
            "-c",
            "mcp_servers={}",
        ]
        .map(String::from)
        .to_vec();
        for feature in SUMMARY_DISABLED_FEATURES {
            args.extend(["--disable".to_string(), feature.to_string()]);
        }
        args.extend(model_args(choice));
        args.push("-".to_string());
        Invocation {
            args,
            stdin: Some(prompt.to_string()),
        }
    }
    fn ask_settings(
        &self,
        outside: &mut dyn Terminal,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>> {
        match Catalog::read() {
            Ok(catalog) => ask_codex(outside, &catalog, current).map(Some),
            Err(error) => {
                crate::interrupt::check()?;
                outside.say(format!("{error:#}\nThe harness settings stay as they are; rerun `thirdshift setup` once codex debug models works."));
                Ok(None)
            }
        }
    }
    fn interpretation(&self, worktree: &Path, _prompt: &str) -> Interpretation {
        Interpretation::new(
            self.name(),
            Box::new(stream::CodexProgress::in_worktree(worktree)),
            Retained::None,
        )
    }
}

/// The arguments that run a Codex session on its Model and Effort, where
/// they are set.
pub fn model_args(choice: &Choice) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = &choice.model {
        args.extend(["-m".to_string(), model.clone()]);
    }
    if let Some(effort) = &choice.effort {
        args.extend([
            "-c".to_string(),
            format!("model_reasoning_effort=\"{effort}\""),
        ]);
    }
    args
}

/// The arguments every session runs `codex` with: `exec`, streaming JSONL,
/// with no approvals and no sandbox (ADR-0012), on the Model and Effort
/// `harness` sets, if any, reading `CLAUDE.md` where a directory has no
/// `AGENTS.md`. With `resume`, the session with that id continues, given
/// every one of those again, as Codex keeps none of them. The prompt comes
/// last.
pub fn codex_args(harness: &Choice, resume: Option<&str>, prompt: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
    ]
    .map(String::from)
    .to_vec();
    args.extend(model_args(harness));
    args.extend(["-c".to_string(), CLAUDE_MD_FALLBACK.to_string()]);
    if let Some(session_id) = resume {
        args.extend(["resume".to_string(), session_id.to_string()]);
    }
    args.push(prompt.to_string());
    args
}

/// The config setting that has Codex read `CLAUDE.md` where a directory has
/// no `AGENTS.md`.
const CLAUDE_MD_FALLBACK: &str = r#"project_doc_fallback_filenames=["CLAUDE.md"]"#;

/// The Codex features a release summary runs without: every tool that could
/// run commands, reach a service, or act on the machine.
const SUMMARY_DISABLED_FEATURES: [&str; 7] = [
    "shell_tool",
    "unified_exec",
    "apps",
    "plugins",
    "browser_use",
    "computer_use",
    "hooks",
];

/// Settle the Model and Effort, if either is named, on Codex's names for
/// them, from its catalog.
fn check_model_and_effort(choice: &mut Choice) -> Result<()> {
    if choice.model.is_none() && choice.effort.is_none() {
        return Ok(());
    }
    progress::step("checking the Model and Effort against codex debug models");
    let settled = Catalog::read()?.settle(choice.model.as_deref(), choice.effort.as_deref())?;
    choice.model = settled.model;
    choice.effort = settled.effort;
    Ok(())
}
/// Ask Codex's Model, listing those in its `catalog`, then its Effort,
/// listing those the Model chosen supports, with `current` as the defaults.
/// Each is matched against the catalog as a Run matches it, and written as
/// Codex names it; one the catalog doesn't have is asked again, with the
/// valid choices.
fn ask_codex(
    outside: &mut dyn Terminal,
    catalog: &Catalog,
    current: &ModelAndEffort,
) -> Result<ModelAndEffort> {
    outside.say(format!(
        "{}'s Models: {}",
        Codex.product_name(),
        catalog.listed().join(", ")
    ));
    let model = ask_checked_setting(
        outside,
        "Model",
        Harness::Codex,
        current.model.as_deref(),
        |model| catalog.settle(model, None).map(|settled| settled.model),
    )?;
    let efforts = catalog.efforts(model.as_deref()).join(", ");
    outside.say(match &model {
        Some(model) => format!("Efforts {model} supports: {efforts}"),
        None => format!(
            "Efforts {}'s Models support: {efforts}",
            Codex.product_name()
        ),
    });
    let effort = ask_checked_setting(
        outside,
        "Effort",
        Harness::Codex,
        current.effort.as_deref(),
        |effort| {
            catalog
                .settle(model.as_deref(), effort)
                .map(|settled| settled.effort)
        },
    )?;
    Ok(ModelAndEffort { model, effort })
}

/// `prompt` as Codex takes it: a first line that loads a Factory skill,
/// `/thirdshift-<skill>` as Claude's prompts write it, loads it as
/// `$thirdshift-<skill>`.
#[cfg(test)]
pub fn codex_prompt(prompt: &str) -> String {
    SkillLoading::Dollar.prompt(prompt)
}
