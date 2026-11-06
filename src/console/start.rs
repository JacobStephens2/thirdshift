//! Requested Ticket/Spec runs: CLI arguments, detached lifetime and observation.
use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use axum::{Extension, Json, response::Response};
use serde::{Deserialize, Serialize};

use super::{api, auth::Viewer};
use crate::{
    config::UserConfig,
    issue::{IssueInput, IssueUrl, Repo},
};

#[derive(Deserialize)]
#[serde(untagged)]
enum Issue {
    Number(u64),
    Text(String),
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum Email {
    Always { address: Option<String> },
    UnlessMerged { address: Option<String> },
    Never,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    repository: String,
    issue: Issue,
    merge: Option<bool>,
    email: Option<Email>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    base_fix: Option<bool>,
    security_review: Option<bool>,
    security_fix: Option<bool>,
}

#[derive(Serialize)]
struct Started {
    version: u8,
    name: String,
}

impl Request {
    fn arguments(&self) -> Result<(Repo, Vec<String>)> {
        let parts: Vec<_> = self.repository.split('/').collect();
        ensure!(
            parts.len() == 2
                && parts.iter().all(|part| !part.is_empty()
                    && *part != "."
                    && *part != ".."
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))),
            "repository must be owner/name"
        );
        let repo = Repo {
            owner: parts[0].into(),
            name: parts[1].into(),
        };
        let input = match &self.issue {
            Issue::Number(number) => IssueInput::parse(&number.to_string())?,
            Issue::Text(text) => IssueInput::parse(text)?,
        };
        let issue = match input {
            IssueInput::Number(number) => IssueUrl::parse(&format!(
                "https://github.com/{}/issues/{number}",
                repo.slug()
            ))?,
            IssueInput::Url(issue) => issue,
        };
        ensure!(
            issue.number > 0
                && issue.matches_origin(&format!("https://github.com/{}", repo.slug())),
            "issue must be a positive number or an Issue URL in {}",
            repo.slug()
        );
        let record_repo = issue.repo();
        let mut args = vec![issue.url];
        for (value, yes, no) in [
            (self.merge, "--merge", "--no-merge"),
            (self.base_fix, "--base-fix", "--no-base-fix"),
            (
                self.security_review,
                "--security-review",
                "--no-security-review",
            ),
            (self.security_fix, "--security-fix", "--no-security-fix"),
        ] {
            if let Some(value) = value {
                args.push(if value { yes } else { no }.into());
            }
        }
        for (flag, value) in [
            ("--harness", &self.harness),
            ("--model", &self.model),
            ("--effort", &self.effort),
        ] {
            if let Some(value) = value {
                args.extend([flag.into(), value.clone()]);
            }
        }
        if let Some(email) = &self.email {
            let (flag, address) = match email {
                Email::Always { address } => ("--email", address.as_ref()),
                Email::UnlessMerged { address } => ("--email-unless-merged", address.as_ref()),
                Email::Never => ("--no-email", None),
            };
            args.push(flag.into());
            if let Some(address) = address {
                args.push(address.clone());
            }
        }
        crate::args::parse(&args)?;
        Ok((record_repo, args))
    }
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    Json(request): Json<Request>,
) -> Response {
    api::action(api::Format::Json, "/", move || start(request, viewer)).await
}

fn start(request: Request, viewer: Viewer) -> Result<Started> {
    let (repo, args) = request.arguments()?;
    let inventory = crate::launch_directories::load()?;
    let directory = inventory
        .directories
        .into_iter()
        .find(|directory| directory.repository.eq_ignore_ascii_case(&repo.slug()))
        .with_context(|| format!("no known Launch directory for {}", repo.slug()))?;
    let config = UserConfig::load()?;
    std::fs::create_dir_all(&config.logs_dir)?;
    let log_path = directory
        .output_log
        .unwrap_or_else(|| config.logs_dir.join("cron.log"));
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut command = Command::new(shell);
    command
        .args([
            "-l",
            "-c",
            "cd -- \"$1\" && shift && exec \"$@\"",
            "thirdshift-start",
        ])
        .arg(&directory.path)
        .arg(std::env::current_exe()?)
        .args(args)
        .current_dir(&directory.path)
        .env(
            crate::logs::Actor::START_ENV,
            serde_json::to_string(&viewer.activity_actor())?,
        )
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and touches no Rust state after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let started_after = chrono::Local::now();
    let mut child = command
        .spawn()
        .context("can't start thirdshift with the login shell")?;
    let result = wait_for_record(&mut child, &config, &repo, &log_path, started_after);
    // Dropping std::process::Child does not reap it. The Console may outlive it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    result.map(|name| Started { version: 1, name })
}

fn wait_for_record(
    child: &mut Child,
    config: &UserConfig,
    repo: &Repo,
    log: &std::path::Path,
    started_after: chrono::DateTime<chrono::Local>,
) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(name) =
            crate::logs::command_for_process(config, repo, child.id(), started_after)?
        {
            return Ok(name);
        }
        if let Some(status) = child.try_wait()? {
            // A very short run can publish and finish between the first read and exit.
            if let Some(name) =
                crate::logs::command_for_process(config, repo, child.id(), started_after)?
            {
                return Ok(name);
            }
            bail!(
                "thirdshift exited with {status} before a Command record appeared; see {}",
                log.display()
            );
        }
        ensure!(
            Instant::now() < deadline,
            "process {} has no Command record after 30 seconds; it may still start: check status and {} before retrying",
            child.id(),
            log.display()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
