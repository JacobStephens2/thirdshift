//! Everything that varies between Harnesses, behind one interface.

use std::path::Path;
use std::process::Child;

use anyhow::Result;

use super::interpretation::Interpretation;
use super::settings::Terminal;
use super::{Choice, Harness, ModelAndEffort, Settings, agy, claude, codex, grok, muse, opencode};

/// One Harness's session protocol and its Model and Effort rules. Callers
/// select it once, then use the same adapter for a session and its Resume.
pub trait Adapter: Sync {
    fn name(&self) -> &'static str;
    fn product_name(&self) -> &'static str;
    fn project_skills(&self) -> &'static str;
    fn skill_loading(&self) -> SkillLoading;
    fn stop_signals(&self) -> &'static [libc::c_int];
    fn settings<'a>(&self, settings: &'a Settings) -> &'a ModelAndEffort;
    fn settings_mut<'a>(&self, settings: &'a mut Settings) -> &'a mut ModelAndEffort;

    /// Arguments for a new session or Resume, and how its prompt reaches
    /// the CLI. The adapter applies its skill-loading style here.
    fn session(&self, choice: &Choice, resume: Option<&str>, prompt: &str) -> Invocation;

    /// A one-shot release summary, interpreted through the same protocol.
    /// Harnesses with a tool-free mode specialize it here.
    fn summary(&self, choice: &Choice, prompt: &str) -> Invocation {
        self.session(choice, None, prompt)
    }

    /// A Security session or its Resume. Guidance reviews do not permit
    /// delegation; other security sessions request fresh sub-agents.
    fn security_session(
        &self,
        choice: &Choice,
        resume: Option<&str>,
        prompt: &str,
        _fresh_sub_agents: bool,
    ) -> Invocation {
        self.session(choice, resume, prompt)
    }
    fn check(&self, choice: &mut Choice) -> Result<()>;
    fn ask_settings(
        &self,
        outside: &mut dyn Terminal,
        current: &ModelAndEffort,
    ) -> Result<Option<ModelAndEffort>>;
    fn interpretation(&self, worktree: &Path, prompt: &str) -> Interpretation;

    /// Stop the session and its descendants, including commands that made
    /// their own process group or session, with this Harness's stop signals.
    fn stop(&self, child: &mut Child) {
        crate::process::stop(child, self.stop_signals());
    }

    /// Fixed overrides, also used by the adapter's Model and Effort check.
    fn environment(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }

    /// Make and exclude an instruction-file fallback link where the CLI
    /// needs one. Native and config-based fallbacks stay in the adapter.
    fn link_instruction_fallback(&self, _worktree: &Path) -> Result<()> {
        Ok(())
    }
}

/// A session's arguments and optional prompt on stdin. `None` keeps stdin
/// null; `Some` is written once and closed while output is drained concurrently.
pub struct Invocation {
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

/// How a Session prompt's first line loads its Factory skill. Prompts are
/// authored in Claude's form so the generated Prompts and skills page stays
/// independent of the selected Harness.
pub enum SkillLoading {
    Slash,
    Dollar,
    Tool,
}

impl SkillLoading {
    pub fn prompt(&self, prompt: &str) -> String {
        let Some(rest) = prompt.strip_prefix("/thirdshift-") else {
            return prompt.to_string();
        };
        match self {
            Self::Slash => prompt.to_string(),
            Self::Dollar => format!("$thirdshift-{rest}"),
            Self::Tool => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let (skill, remaining) = rest.split_at(end);
                format!("Load thirdshift-{skill} with your skill tool.{remaining}")
            }
        }
    }
}

/// Registration comes from the adapters, including Setup's order.
pub const REGISTERED: [(Harness, &dyn Adapter); 6] = [
    claude::REGISTRATION,
    codex::REGISTRATION,
    agy::REGISTRATION,
    grok::REGISTRATION,
    muse::REGISTRATION,
    opencode::REGISTRATION,
];
