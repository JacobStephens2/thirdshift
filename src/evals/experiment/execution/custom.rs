//! Version-1 Custom runner requests and per-Trial result classification.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Output as ProcessOutput;
use std::time::Instant;

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::json;

use super::super::{Task, arms, store};
use crate::process::{self, Control, Interruption};
use crate::trial::{Outcome, Output, VoidCause};

mod recording;
pub(crate) mod sessions;
pub(super) use recording::Recorder;

pub(super) fn collect(
    recorder: &mut Recorder,
    runner: &arms::Runner,
    traffic: &crate::proxy::Accounting,
    output: &mut Output,
    signal: &Path,
) -> Result<()> {
    match recorder.collect() {
        Ok(imported) => {
            if let Some(limit) = recorder.limit.as_ref().or(imported.usage_limit.as_ref()) {
                crate::session::limits::notify(signal, limit)?;
                output.outcome = Outcome::Void;
                output.void_cause = Some(VoidCause::UsageLimit);
                output.cause = Some(crate::session::limits::Reached(limit.clone()).to_string());
            } else if recorder.web_search || imported.web_search {
                output.outcome = Outcome::Void;
                output.void_cause = Some(VoidCause::WebSearch);
                output.cause = Some("Session transcript contains a web-search tool call".into());
            } else if output.outcome != Outcome::Void {
                let sessions = recorder.sessions + imported.sessions;
                let harness_sessions = recorder.sessions + imported.harness_sessions;
                let crossed: Vec<_> = traffic
                    .harness_invocations
                    .iter()
                    .filter(|run| run.provider_connections > 0)
                    .collect();
                let declared =
                    !runner.manifest.harnesses.is_empty() || !runner.manifest.providers.is_empty();
                if (sessions == 0
                    && (traffic.provider_connections > 0
                        || (declared
                            && matches!(
                                output.outcome,
                                Outcome::Delivered | Outcome::StoppedToAsk
                            ))))
                    || harness_sessions < crossed.len()
                {
                    output.outcome = Outcome::Void;
                    output.void_cause = Some(VoidCause::AccountingGap);
                    output.cause = Some(format!(
                        "accounting gap: {sessions} recorded sessions, {harness_sessions} Harness sessions, {} provider connections, {} Harness runs across the proxy ({})",
                        traffic.provider_connections,
                        crossed.len(),
                        crossed
                            .iter()
                            .map(|run| run.harness.name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        Err(error) if output.outcome != Outcome::Void => {
            output.outcome = Outcome::Void;
            output.void_cause = Some(VoidCause::AccountingGap);
            output.cause = Some(format!(
                "accounting gap: invalid Custom runner Session evidence: {error:#}"
            ));
        }
        Err(_) => {}
    }
    Ok(())
}

/// Inspect the normalized facts, shared by native and Custom-runner Trials.
pub(super) fn web_search(attempt: &Path) -> Result<bool> {
    use crate::otlp_jsonl::{AnyValue, Logs, Reader};
    for session in
        super::super::trial_sessions::attempt(&super::super::trial_results::files::Native, attempt)?
            .sessions
    {
        let Some(path) = session.logs else { continue };
        for request in Reader::<_, Logs>::new(std::io::BufReader::new(fs::File::open(path)?)) {
            for record in request?
                .resource_logs
                .into_iter()
                .flat_map(|resource| resource.scope_logs)
                .flat_map(|scope| scope.log_records)
            {
                for attribute in record.attributes {
                    if attribute.key == "thirdshift.events"
                        && let Some(AnyValue::String(events)) = attribute.value
                    {
                        // Decode through Value, as the other trace readers do:
                        // arbitrary_precision numbers otherwise pass through
                        // serde's tagged-enum buffer as maps rather than f64.
                        let events =
                            serde_json::from_str::<Vec<serde_json::Value>>(&events)?
                                .into_iter()
                                .map(serde_json::from_value)
                                .collect::<serde_json::Result<
                                    Vec<crate::harness::interpretation::event::Event>,
                                >>()?;
                        if sessions::web_search(&events) {
                            return Ok(true);
                        }
                    }
                }
            }
        }
    }
    Ok(false)
}

pub(super) fn prepare(
    runner: &arms::Runner,
    task: &Task,
    repository: &Path,
    attempt: &Path,
    record_command: &[String],
) -> Result<()> {
    let runner_dir = runner.install(attempt)?;
    let inputs = task
        .manifest
        .trial
        .as_ref()
        .context("missing Trial inputs")?;
    fs::create_dir_all(attempt.join("session"))?;
    fs::create_dir_all(attempt.join("output"))?;
    let git = crate::git::Git::new(repository);
    crate::skills::exclude(&git, &format!("/{}", crate::prompt::ISSUE_FILE))?;
    let exclude = git.run(&["rev-parse", "--git-path", "info/exclude"])?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(repository.join(exclude))?;
    writeln!(file)?;
    for pattern in &runner.manifest.git_exclude {
        writeln!(file, "{pattern}")?;
    }
    store::write(
        &attempt.join("request.json"),
        &json!({
            "version": 1, "repository": repository, "issue_file": repository.join(crate::prompt::ISSUE_FILE),
            "issue_url": inputs.issue_url, "base": inputs.base, "branch": inputs.branch,
            "runner_dir": runner_dir, "output_dir": attempt.join("output"),
            "record_command": record_command
        }),
    )
}

pub(super) fn command(runner: &arms::Runner, attempt: &Path) -> Result<Vec<String>> {
    let directory = attempt.join("runner");
    let directory = directory
        .to_str()
        .context("runner directory must be UTF-8")?;
    let mut command: Vec<_> = runner
        .manifest
        .command
        .iter()
        .map(|arg| arg.replace("{runner_dir}", directory))
        .collect();
    command.push(
        attempt
            .join("request.json")
            .to_str()
            .context("runner request must be UTF-8")?
            .into(),
    );
    Ok(command)
}

/// Argv for the scope's program. `env -i` applies to the runner, after
/// `--oom-exec`, so the scope launcher can still reach the user bus. The
/// shell only `exec`s the runner; assignments stay in the allowlist, including
/// a program path that itself contains `=`.
pub(super) fn confined(argv: Vec<String>, environment: &[(String, String)]) -> Result<Vec<String>> {
    ensure!(!argv.is_empty(), "Custom runner command is empty");
    let mut command = Vec::with_capacity(environment.len() + argv.len() + 6);
    command.push("/usr/bin/env".into());
    command.push("-i".into());
    command.push("--".into());
    for (name, value) in environment {
        ensure!(
            portable_name(name),
            "Custom runner environment name is not portable"
        );
        ensure!(
            !value.chars().any(|c| matches!(c, '\0' | '\n' | '\r')),
            "Custom runner environment value must be a single line"
        );
        command.push(format!("{name}={value}"));
    }
    command.push("/bin/sh".into());
    command.push("-c".into());
    command.push(r#"exec "$0" "$@""#.into());
    command.extend(argv);
    Ok(command)
}

fn portable_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Operator locale tokens safe to pass through. Anything else can name a
/// file or carry a value that is not a locale.
pub(super) fn locale_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '+' | '-'))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultDocument {
    version: u64,
    outcome: Outcome,
    #[serde(default)]
    void_cause: Option<VoidCause>,
    #[serde(default)]
    cause: Option<String>,
}

/// Runner mistakes fail one Trial. Outer supervision classifies its own
/// evidence before this function, and may override it after collection.
pub(super) fn result(process: &ProcessOutput, attempt: &Path, elapsed: u128) -> Output {
    let result = (|| -> Result<Output> {
        let document: ResultDocument = serde_json::from_slice(
            &fs::read(attempt.join("output/result.json"))
                .context("missing or unreadable Custom runner result.json")?,
        )
        .context("invalid Custom runner result.json")?;
        ensure!(
            document.version == 1,
            "unsupported Custom runner result version"
        );
        ensure!(
            document.outcome != Outcome::Succeeded,
            "Custom runner outcome must be delivered, stopped_to_ask, failed or void"
        );
        if let Some(cause) = document.void_cause {
            ensure!(
                matches!(
                    cause,
                    VoidCause::SessionUnavailable | VoidCause::UsageLimit | VoidCause::RateLimit
                ),
                "Custom runner claimed outer void_cause {cause}"
            );
            ensure!(
                document.outcome == Outcome::Void,
                "void_cause requires a void outcome"
            );
        }
        ensure!(
            document.outcome != Outcome::Void || document.void_cause.is_some(),
            "Custom runner void requires void_cause"
        );
        ensure!(
            process.status.success(),
            "Custom runner exited nonzero without outer Void evidence"
        );
        let mut output = Output::failed(elapsed, "Custom runner failed");
        output.outcome = document.outcome;
        output.void_cause = document.void_cause;
        output.cause = document.cause;
        if output.outcome == Outcome::Failed {
            output.cause = Some(diagnostic(
                process,
                output.cause.as_deref().unwrap_or("Custom runner failed"),
            ));
        }
        Ok(output)
    })();
    result.unwrap_or_else(|error| {
        Output::failed(elapsed, &diagnostic(process, &format!("{error:#}")))
    })
}

fn diagnostic(process: &ProcessOutput, cause: &str) -> String {
    format!(
        "{cause}; exit status: {}; stderr: {}",
        process.status,
        String::from_utf8_lossy(&process.stderr)
    )
}

pub(super) fn inspect(
    runner: &arms::Runner,
    repository: &Path,
    attempt: &Path,
    deadline: Instant,
) -> Result<Option<String>> {
    let paths = crate::evals::repository::bytes_until(
        repository,
        &["ls-tree", "-rz", "--name-only", "HEAD"],
        deadline,
    )?;
    // Evaluate only the frozen manifest's patterns, independent of the
    // runner's edits to .gitignore, its index or .git/info/exclude.
    let scratch = tempfile::tempdir()?;
    crate::git::Git::new(scratch.path()).run(&["init", "-q", "--template="])?;
    fs::create_dir_all(scratch.path().join(".git/info"))?;
    fs::write(
        scratch.path().join(".git/info/exclude"),
        runner.manifest.git_exclude.join("\n") + "\n",
    )?;
    let matches = process::output_until(
        std::process::Command::new("git")
            .current_dir(scratch.path())
            .args([
                "-c",
                "core.excludesFile=/dev/null",
                "check-ignore",
                "--no-index",
                "-z",
                "--stdin",
            ]),
        Some(&paths),
        Control {
            name: "Custom runner HEAD check",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
        deadline,
        &|| Ok(()),
    )?;
    ensure!(
        matches.status.code() == Some(1) || matches.status.success(),
        "could not check Custom runner git_exclude: {}",
        String::from_utf8_lossy(&matches.stderr)
    );
    ensure!(
        matches.stdout.is_empty(),
        "Custom runner committed a git_exclude path on HEAD: {}",
        String::from_utf8_lossy(&matches.stdout).replace('\0', ", ")
    );
    let status = crate::evals::repository::bytes_until(
        repository,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--ignored=traditional",
        ],
        deadline,
    )?;
    if status.is_empty() {
        return Ok(None);
    }
    fs::write(attempt.join("uncommitted.txt"), &status)?;
    Ok(Some(format!(
        "uncommitted working-tree files ignored by grading: {}",
        String::from_utf8_lossy(&status).trim()
    )))
}
