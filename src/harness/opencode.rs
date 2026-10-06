//! OpenCode: private standalone sessions, stdin prompts and an authoritative export.
mod export;
mod stream;

use super::adapter::{Adapter, Invocation, SkillLoading};
use super::{Choice, Harness, ModelAndEffort, Settings, said};
use crate::progress::Stream;
use crate::setup::{Outside, questions::ask_setting};
use anyhow::{Context, Result, anyhow, bail};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::Duration;

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
        outside: &mut dyn Outside,
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
            match outside.opencode_check(&chosen) {
                Ok(()) => return Ok(Some(chosen)),
                Err(error) => outside.say(format!("{error:#}")),
            }
        }
    }
    fn stream(&self, _worktree: &Path, prompt: &str) -> Box<dyn Stream> {
        Box::new(stream::OpenCodeProgress::for_prompt(prompt))
    }
    fn link_instruction_fallback(&self, worktree: &Path) -> Result<()> {
        super::instructions::link_fallback(worktree, "AGENTS.md", &[])
    }
    fn read_after_exit(&self, worktree: &Path, stream: Box<dyn Stream>) -> Box<dyn Stream> {
        export::read_after_exit(worktree, stream)
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
    let mut child = Command::new(OpenCode.name())
        .args(invocation.args)
        .envs(OpenCode.environment().iter().copied())
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not run opencode to check the Model and Effort")?;
    let written = child
        .stdin
        .take()
        .context("no stdin from opencode")?
        .write_all(invocation.stdin.as_deref().unwrap_or("").as_bytes());
    if let Err(error) = written {
        OpenCode.stop(&mut child);
        return Err(error).context("could not write the check prompt to opencode");
    }
    let output = check_output(&mut child)?;
    let mut stream = OpenCode.stream(Path::new("."), "Reply with OK.");
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        stream.condense(line);
    }
    let stream = OpenCode.read_after_exit(Path::new("."), stream);
    if !output.status.success() || stream.failed() {
        bail!(
            "opencode refused the Model or Effort in its test call: {}",
            stream
                .error()
                .map(String::from)
                .unwrap_or_else(|| said(&output))
        );
    }
    Ok(())
}

/// OpenCode errors appear as strings or typed objects in its event/export formats.
fn error_text(error: &serde_json::Value) -> Option<&str> {
    error.as_str().or_else(|| error["message"].as_str())
}

/// Drain both pipes while watching for interruption, so a slow check cannot
/// strand its private server or commands before a Run has even started work.
fn check_output(child: &mut Child) -> Result<Output> {
    fn read(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<std::io::Result<Vec<u8>>> {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    }
    let stdout = read(child.stdout.take().context("no stdout from opencode")?);
    let stderr = read(child.stderr.take().context("no stderr from opencode")?);
    let status = loop {
        if crate::interrupt::requested() {
            OpenCode.stop(child);
            let _ = stdout.join();
            let _ = stderr.join();
            bail!("interrupted");
        }
        if let Some(status) = child
            .try_wait()
            .context("could not wait for opencode's test call")?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    Ok(Output {
        status,
        stdout: stdout
            .join()
            .map_err(|_| anyhow!("OpenCode's check stdout reader panicked"))??,
        stderr: stderr
            .join()
            .map_err(|_| anyhow!("OpenCode's check stderr reader panicked"))??,
    })
}
