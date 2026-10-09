//! Release summaries reuse the User config's Harness and its session protocol,
//! away from the maintainer's checkout, without installing Factory skills.

use std::process::Command;

use anyhow::{Context, Result, bail};

use super::{Asked, Choice, Settings, process};
use crate::progress;

/// Return only the final assistant text, or fail so the release script can
/// fall back to GitHub's generated notes. Never start Setup or a Run.
pub fn write(settings: &Settings, prompt: &str) -> Result<String> {
    let mut choice = Choice::of(&Asked::default(), settings);
    choice.check()?;
    progress::step(format_args!("writing the release summary with {choice}"));
    let directory = tempfile::tempdir().context("could not prepare the summary directory")?;
    let prompt_file = directory.path().join("release-summary.md");
    std::fs::write(&prompt_file, prompt).context("could not write the summary input")?;
    let adapter = choice.harness.adapter();
    let invocation = adapter.summary(&choice, prompt, &prompt_file);
    let output = process::output(
        adapter,
        Command::new(adapter.name())
            .args(invocation.args)
            .current_dir(directory.path()),
        invocation.stdin.as_deref(),
    )
    .context("could not run the release summary Harness")?;
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let mut interpretation = adapter.interpretation(directory.path(), prompt);
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        interpretation.condense(line);
    }
    let ended = interpretation.finish(Ok(output.status)).outcome?;
    if !ended.killed.is_empty() {
        bail!(
            "the summary ended with unfinished background work: {}",
            ended.killed.join("; ")
        );
    }
    ended
        .final_message
        .filter(|text| !text.trim().is_empty())
        .context("the release summary Harness returned no summary")
}
