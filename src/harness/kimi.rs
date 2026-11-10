//! Kimi Code CLI: `kimi -p` sessions with a `stream-json` stream, whose usage
//! comes from the session's own `wire.jsonl` under `KIMI_CODE_HOME`.

mod catalog;
mod stream;
mod wire;

pub use catalog::Catalog;

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::interpretation::Interpretation;
use super::settings::{Terminal, ask_checked_setting};
use super::{Choice, Harness, ModelAndEffort, Settings};

pub struct Kimi;
pub const REGISTRATION: (Harness, &dyn Adapter) = (Harness::Kimi, &Kimi);

/// The variable naming Kimi Code's home, which holds its config, sign-in
/// and sessions.
pub(crate) const HOME_ENV: &str = "KIMI_CODE_HOME";
/// Kimi Code's home relative to the user's, when `KIMI_CODE_HOME` is unset.
pub(crate) const HOME_NAME: &str = ".kimi-code";
/// What a private or sandboxed home copies from Kimi Code's home: its config,
/// the managed provider's sign-in and the user's instructions.
pub(crate) const HOME_FILES: [&str; 3] = ["config.toml", "credentials/kimi-code.json", "AGENTS.md"];

/// Kimi Code's home for a process launched with `launched` as its `HOME`, or
/// with this process's environment when the launch keeps it: the one each
/// session is pointed at, and where its `wire.jsonl` is read back.
pub(crate) fn home(launched: Option<&Path>) -> Option<PathBuf> {
    if let Some(home) = launched {
        return Some(home.join(HOME_NAME));
    }
    match std::env::var_os(HOME_ENV).filter(|home| !home.is_empty()) {
        Some(home) => Some(PathBuf::from(home)),
        None => crate::config::home().ok().map(|home| home.join(HOME_NAME)),
    }
}

impl Adapter for Kimi {
    fn name(&self) -> &'static str {
        "kimi"
    }
    fn product_name(&self) -> &'static str {
        "Kimi Code"
    }
    fn project_skills(&self) -> &'static str {
        ".agents/skills"
    }
    /// Print mode runs a prompt as written, so the model loads the skill
    /// with its `Skill` tool.
    fn skill_loading(&self) -> SkillLoading {
        SkillLoading::Tool
    }
    fn stop_signals(&self) -> &'static [libc::c_int] {
        &[libc::SIGTERM]
    }
    /// The worktree is trusted, as on every Harness (ADR-0013), which lets
    /// its project MCP servers start; updates and telemetry stay off.
    fn environment(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("KIMI_CODE_TRUST_WORKSPACE", "1"),
            ("KIMI_CODE_NO_AUTO_UPDATE", "1"),
            ("KIMI_DISABLE_TELEMETRY", "1"),
        ]
    }
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort {
        &settings.kimi
    }
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort {
        &mut settings.kimi
    }
    /// Print mode approves every action itself; `--yolo` and `--auto` are
    /// refused beside `-p`.
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation {
        let mut args: Vec<String> = ["--output-format", "stream-json"]
            .map(String::from)
            .to_vec();
        if let Some(model) = &choice.model {
            args.extend(["-m".to_string(), model.clone()]);
        }
        if let Some(id) = resume {
            args.extend(["--session".to_string(), id.to_string()]);
        }
        args.extend(["-p".to_string(), self.skill_loading().prompt(prompt)]);
        Invocation { args, stdin: None }
    }
    /// An agent whose `tools` list is empty, so no tool runs: the release
    /// input is its instructions, in a file beside `prompt_file`, and `-p`
    /// only asks for the summary. A file that can't be written fails the
    /// session, and the release falls back to generated notes.
    fn summary(&self, choice: &Choice, prompt: &str, prompt_file: &Path) -> Invocation {
        let agent = prompt_file.with_file_name("kimi-release-summary.md");
        let written = std::fs::write(
            &agent,
            format!(
                "---\nname: release-summary\ndescription: Writes a release summary.\ntools: []\n---\n\n{prompt}\n"
            ),
        );
        if let Err(error) = written {
            crate::progress::step(format!(
                "warning: could not write {}: {error}",
                agent.display()
            ));
        }
        let mut invocation = self.session(
            choice,
            None,
            "Write the release summary your instructions describe.",
        );
        // Before `-p` and its prompt, which always come last.
        let at = invocation.args.len() - 2;
        invocation.args.splice(
            at..at,
            [
                "--agent-file".to_string(),
                agent.to_string_lossy().into_owned(),
            ],
        );
        invocation
    }
    fn check(&self, choice: &mut Choice, _probe: super::Probe) -> Result<()> {
        if choice.effort.is_some() {
            bail!(
                "Kimi Code takes no Effort from the command line: leave effort blank, \
                 and set thinking in Kimi Code's config"
            );
        }
        crate::progress::step("checking the Model against kimi provider list");
        choice.model = Catalog::read()?.settle(choice.model.as_deref())?;
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
                outside.say(format!("{error:#}\nThe harness settings stay as they are; rerun `thirdshift setup` once kimi provider list works."));
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
            Harness::Kimi,
            current.model.as_deref(),
            |model| catalog.settle(model),
        )?;
        Ok(Some(ModelAndEffort {
            model,
            effort: None,
        }))
    }
    fn interpretation(&self, _worktree: &Path, prompt: &str) -> Interpretation {
        Interpretation::new(
            self.name(),
            Box::new(stream::KimiProgress::for_prompt(prompt)),
        )
        .with_retained(Box::new(wire::Completion::new(
            self.skill_loading().prompt(prompt),
        )))
    }
    fn stream_version<'a>(&self, event: &'a serde_json::Value) -> Option<&'a str> {
        (event["role"] == "meta" && event["type"] == "system.version")
            .then(|| event["version"].as_str())
            .flatten()
    }
    /// Kimi Code reads `AGENTS.md`, never `CLAUDE.md`.
    fn link_instruction_fallback(&self, worktree: &Path) -> Result<()> {
        super::instructions::link_fallback(worktree, "AGENTS.md", &[])
    }
    /// Point Kimi at the home its `wire.jsonl` is read back from: under the
    /// launch's own `HOME` when it sets one, as a private home does.
    fn prepare(&self, command: &mut Command) {
        let launched = command
            .get_envs()
            .find_map(|(name, value)| (name == "HOME").then_some(value).flatten())
            .map(PathBuf::from);
        if let Some(home) = home(launched.as_deref()) {
            command.env(HOME_ENV, home);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::ChosenBy;

    fn choice(model: Option<&str>) -> Choice {
        Choice {
            harness: Harness::Kimi,
            model: model.map(String::from),
            effort: None,
            chosen_by: ChosenBy::Command,
        }
    }

    #[test]
    fn a_resume_uses_the_hinted_session_id_with_the_same_flags_and_null_stdin() {
        let mut progress = Kimi.interpretation(Path::new("/repo"), "");
        progress.condense(r#"{"role":"meta","type":"session.resume_hint","session_id":"ses_1","command":"kimi -r ses_1"}"#);
        let ended = crate::harness::interpretation_tests::finish(progress)
            .outcome
            .unwrap();
        let resume = Kimi.session(
            &choice(Some("kimi-k3.1-pro")),
            ended.session_id.as_deref(),
            "/thirdshift-implement issue-url",
        );
        assert_eq!(
            resume.args,
            [
                "--output-format",
                "stream-json",
                "-m",
                "kimi-k3.1-pro",
                "--session",
                "ses_1",
                "-p",
                "Load thirdshift-implement with your skill tool. issue-url"
            ]
        );
        assert_eq!(resume.stdin, None);
        assert_eq!(
            Kimi.environment(),
            [
                ("KIMI_CODE_TRUST_WORKSPACE", "1"),
                ("KIMI_CODE_NO_AUTO_UPDATE", "1"),
                ("KIMI_DISABLE_TELEMETRY", "1")
            ]
        );
    }

    #[test]
    fn the_summary_runs_an_agent_with_no_tools_whose_instructions_are_the_release_input() {
        let directory = tempfile::tempdir().unwrap();
        let prompt_file = directory.path().join("release-summary.md");
        let summary = Kimi.summary(&choice(None), "Summarize #1 and #2.", &prompt_file);
        let agent = directory.path().join("kimi-release-summary.md");
        assert_eq!(
            summary.args,
            [
                "--output-format",
                "stream-json",
                "--agent-file",
                agent.to_str().unwrap(),
                "-p",
                "Write the release summary your instructions describe."
            ]
        );
        let written = std::fs::read_to_string(agent).unwrap();
        assert!(written.contains("\ntools: []\n"), "{written}");
        assert!(written.ends_with("\nSummarize #1 and #2.\n"), "{written}");
    }

    #[test]
    fn an_effort_fails_before_the_catalog_is_read() {
        let mut chosen = Choice {
            effort: Some("high".into()),
            ..choice(None)
        };
        assert!(
            Kimi.check(&mut chosen, super::super::Probe::Allow)
                .unwrap_err()
                .to_string()
                .contains("Kimi Code takes no Effort")
        );
    }

    #[test]
    fn a_launch_with_its_own_home_points_kimi_under_it() {
        let mut command = Command::new("kimi");
        command.env("HOME", "/private/home");
        Kimi.prepare(&mut command);
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == HOME_ENV)
                .and_then(|(_, value)| value),
            Some(Path::new("/private/home/.kimi-code").as_os_str())
        );
    }

    #[test]
    fn claude_only_instructions_are_linked_as_agents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(dir.path().join("CLAUDE.md"), "rules\n").unwrap();
        Kimi.link_instruction_fallback(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_link(dir.path().join("AGENTS.md")).unwrap(),
            Path::new("CLAUDE.md")
        );
    }

    #[test]
    fn a_turn_without_the_resume_hint_still_reports_its_wire_usage() {
        let home = tempfile::tempdir().unwrap();
        let agent = home.path().join(".kimi-code/sessions/ws/ses_1/agents/main");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::write(
            agent.join("wire.jsonl"),
            include_str!("../../tests/fixtures/kimi-wire.jsonl"),
        )
        .unwrap();
        // The record's user message is this prompt as Kimi was given it.
        let mut progress = Kimi.interpretation(Path::new("/repo"), "/thirdshift-implement");
        progress.set_home(home.path());
        progress.condense(r#"{"role":"assistant","content":"partial"}"#);
        let finished = crate::harness::interpretation_tests::finish(progress);
        assert!(
            finished.report.unwrap().summary.is_some(),
            "a turn that ended without session.resume_hint lost its wire.jsonl usage"
        );
    }

    #[test]
    fn the_version_comes_from_the_streams_system_version_line() {
        let line = serde_json::json!({"role":"meta","type":"system.version","version":"2.5.2"});
        assert_eq!(Kimi.stream_version(&line), Some("2.5.2"));
        assert_eq!(
            Kimi.stream_version(&serde_json::json!({"version":"1"})),
            None
        );
    }
}
