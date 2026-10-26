//! One headless skill session, outside the Run/Command logging lifecycle.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

use crate::config::UserConfig;
use crate::git::Git;
use crate::harness::{Asked, Choice, process};
use crate::issue::IssueInput;
use crate::worktree::SkillWorktree;
use crate::{delivery, interrupt, launch, progress, prompt, skills};

pub fn run(name: &str, issue: Option<IssueInput>) -> Result<String> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let issue = issue.map(launch::resolve_issue).transpose()?;
    if let Some(issue) = &issue {
        // Same rule as a Run: a foreign Issue URL must not start a session.
        let origin = Git::new(std::env::current_dir()?).run(&["config", "remote.origin.url"])?;
        if !issue.matches_origin(&origin) {
            bail!(
                "origin mismatch: {} is not in the repository at origin {origin}",
                issue.url
            );
        }
    }
    let worktree = SkillWorktree::create(&Git::new(std::env::current_dir()?))?;
    let result = (|| {
        let skill = skills::find(worktree.path(), name)?;
        let mut choice = Choice::of(&Asked::default(), &config.harness);
        choice.check_without_probe()?;
        let adapter = choice.harness.adapter();
        skills::link_into(worktree.path(), adapter)?;
        adapter.link_instruction_fallback(worktree.path())?;
        if let Some(issue) = &issue {
            delivery::hand_over_issue(issue, worktree.path())?;
        }
        let prompt = prompt::skill(&skill.name, &skill.path, issue.as_ref());
        crate::resources::wait()?;
        progress::step(format_args!("running {} with {choice}", skill.name));
        run_session(worktree.path(), &choice, "the skill session", &prompt)
    })();
    skills::remove_written();
    result
}

/// One headless session in `worktree` on `choice`. `label` names the session
/// in a missing-final-message error. The caller waits for host resources.
pub(crate) fn run_session(
    worktree: &Path,
    choice: &Choice,
    label: &str,
    prompt: &str,
) -> Result<String> {
    let adapter = choice.harness.adapter();
    let invocation = adapter.session(choice, None, prompt);
    let executed = process::streaming(
        adapter,
        Command::new(adapter.name())
            .args(invocation.args)
            .current_dir(worktree),
        invocation.stdin.as_deref(),
        None,
        adapter.interpretation(worktree, prompt),
        |output, interpretation| {
            for line in BufReader::new(output).lines() {
                for progress in interpretation.condense(&line?) {
                    progress::step(progress);
                }
            }
            Ok(())
        },
    )?;
    let ended = executed.state.finish(executed.execution).outcome?;
    ended.require_final_message(label)
}
