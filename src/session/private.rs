//! Short private analysis sessions share sandbox, caps, native quota location
//! and recording. Their callers own the input packet and response validation.
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};

use super::recording::{Input, Recording};
use crate::harness::Choice;
use crate::scope::Caps;

pub(crate) struct Request<'a> {
    pub kind: &'a str,
    pub choice: &'a Choice,
    pub binary: &'a Path,
    pub workspace: &'a Path,
    pub artifacts: &'a Path,
    pub prompt: &'a str,
    pub timeout: Duration,
    pub caps: &'a Caps,
    pub environment: &'a [(&'a str, &'a Path)],
}

pub(crate) fn run(request: Request<'_>) -> Result<()> {
    crate::resources::wait()?;
    let Request {
        kind,
        choice,
        binary,
        workspace,
        artifacts,
        prompt,
        timeout,
        caps,
        environment,
    } = request;
    let adapter = choice.harness.adapter();
    let invocation = adapter.trial_session(choice, None, prompt);
    let sandbox = crate::sandbox::for_session(choice, binary, workspace, artifacts)?;
    let launch = sandbox.as_ref().map(|prepared| prepared.launch());
    // The scope encloses the sandbox and all tools. Native quota records
    // live beside the sandbox's login, rather than in the host's home.
    let mut launched = match &launch {
        Some(launch) => launch.scoped_command(&invocation.args, workspace, caps)?,
        None => crate::scope::grader_command(binary, &invocation.args, workspace, caps)?,
    };
    let mut interpretation = adapter.interpretation(workspace, prompt);
    if let Some(launch) = &launch
        && let Some((_, home)) = launch.env.iter().find(|(name, _)| name == "HOME")
    {
        interpretation.set_home(Path::new(home));
    }
    launched
        .command
        .stderr(fs::File::create(artifacts.join("stderr"))?);
    for (name, path) in environment {
        launched.command.env(name, path);
    }
    let scope = launched.scope;
    fs::write(artifacts.join("scope"), scope.unit())?;
    let ended = Recording::for_trial().run(
        kind,
        choice,
        &mut launched.command,
        Input {
            prompt,
            stdin: invocation.stdin.as_deref(),
            deadline: Some(Instant::now() + timeout),
            private: true,
        },
        &artifacts.join("stdout.jsonl"),
        interpretation,
    );
    crate::interrupt::check()?;
    let ended = ended.map_err(|error| {
        // Preserve the shared stop signal so Scoring and Experiments apply
        // their own pause policy. Other diagnostics stay in private artifacts.
        if error.is::<super::limits::Reached>() {
            error
        } else if scope.oom_killed() {
            anyhow::anyhow!("{kind} hit its memory cap ({}); see private Grader artifacts, and raise the trial.memory_max User config to retry", crate::scope::OOM_KILL_CAUSE)
        } else {
            anyhow::anyhow!("{kind} session failed; see private Grader artifacts")
        }
    })?;
    ensure!(
        ended.killed.is_empty(),
        "{kind} left unfinished background work"
    );
    Ok(())
}
