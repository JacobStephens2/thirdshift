//! One headless skill session, outside the Run/Command logging lifecycle.

use std::io::{BufRead, BufReader};
use std::process::Command;

use anyhow::Result;

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
        progress::step(format_args!("running {} with {choice}", skill.name));
        let invocation = adapter.session(&choice, None, &prompt);
        let executed = process::streaming(
            adapter,
            Command::new(adapter.name())
                .args(invocation.args)
                .current_dir(worktree.path()),
            invocation.stdin.as_deref(),
            adapter.interpretation(worktree.path(), &prompt),
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
        ended.require_final_message("the skill session")
    })();
    skills::remove_written();
    result
}
