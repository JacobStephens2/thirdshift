//! One tool-less summary Session. No Launch directory or host home is mounted.
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::git::Git;
use crate::github::GitHub;
use crate::harness::Choice;
use crate::issue::Repo;
use crate::session::{
    Kind, Logged,
    recording::{Input, Policy, Recording},
};
use crate::{logs, sandbox};

pub(super) fn input(
    git: &Git,
    github: &GitHub,
    base: &str,
    previous: Option<&str>,
    version: &str,
    diff: &str,
) -> Result<String> {
    let range = previous.map_or_else(|| base.to_owned(), |previous| format!("{previous}..{base}"));
    let subjects = git.run(&["log", "--reverse", "--merges", "--format=%s", &range])?;
    let mut pull_requests = Vec::new();
    for subject in subjects.lines() {
        if let Some(number) = subject
            .strip_prefix("Merge pull request #")
            .and_then(|rest| rest.split_once(' ').map(|(number, _)| number))
            && number.bytes().all(|byte| byte.is_ascii_digit())
        {
            pull_requests.push(github.release_pr_input(number)?);
        }
    }
    Ok(crate::prompt::release_summary(
        version,
        diff,
        &pull_requests,
    ))
}

pub(super) fn write(
    choice: &mut Choice,
    repo: &Repo,
    prompt: &str,
    trace: &mut Option<PathBuf>,
) -> Result<String> {
    choice.check_without_probe()?;
    crate::resources::wait()?;
    let root = tempfile::Builder::new()
        .prefix("thirdshift-release-summary-")
        .tempdir()?;
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace)?;
    let prompt_file = workspace.join("release-summary.md");
    fs::write(&prompt_file, prompt)?;
    let adapter = choice.harness.adapter();
    // prepare, rather than for_trial/for_session: a release never honors the
    // Trial-only THIRDSHIFT_TRIAL_NO_SANDBOX escape hatch.
    let prepared = sandbox::prepare(&sandbox::Request {
        harness: choice.harness,
        model: choice.model.as_deref(),
        repository: &workspace,
        skills_dir: &crate::skills::written_out()?,
        harness_binary: &sandbox::resolve_binary(adapter.name())?,
        home: &crate::config::home()?,
        fresh_home: &root.path().join("home"),
        secrets: crate::resend_key::Credentials::read()
            .map(|credentials| sandbox::Secrets::from(&credentials))
            .unwrap_or_default(),
        adapter_env: adapter.environment().to_vec(),
    })?;
    let mut launch = prepared.launch();
    for (name, value) in adapter.summary_environment() {
        launch.set_env(name, value);
    }
    let invocation = adapter.summary(choice, prompt, &prompt_file);
    let mut command = launch.session_command(&invocation.args, false)?;
    command.command().current_dir(&workspace);
    let recording = Recording::capture();
    let kind = Logged::of(Kind::ReleaseSummary);
    let path = recording.session_path(&logs::session_dir(repo), "release", &kind.token())?;
    logs::session(kind, &path);
    *trace = Some(path.clone());
    let ended = recording.run(
        &kind.token(),
        choice,
        command.as_mut(),
        Input {
            prompt,
            policy: Policy::StreamOnly,
            stdin: invocation.stdin.as_deref(),
            ..Default::default()
        },
        &path,
    )?;
    ended
        .require_final_message("the release summary")
        .context("release summary Harness returned empty output")
}
