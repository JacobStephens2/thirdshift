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
    asks::Flags,
    base_fix::BaseFixAsk,
    config::UserConfig,
    harness::{self, Harness, ModelAndEffort},
    issue::{IssueInput, IssueUrl, Repo},
    notification::NotificationAsk,
    run::Goal,
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

impl Email {
    fn ask(&self) -> Result<NotificationAsk> {
        let address = |address: &Option<String>| -> Result<Option<String>> {
            if let Some(address) = address {
                ensure!(
                    crate::args::is_address(address),
                    "email address must be an address, not a CLI option"
                );
            }
            Ok(address.clone())
        };
        Ok(match self {
            Self::Always { address: to } => NotificationAsk::Send(address(to)?),
            Self::UnlessMerged { address: to } => NotificationAsk::SendUnlessMerged(address(to)?),
            Self::Never => NotificationAsk::Skip,
        })
    }
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
        crate::args::public_flag_words(&self.flags()?, &mut args);
        crate::args::parse(&args)?;
        Ok((record_repo, args))
    }

    fn flags(&self) -> Result<Flags> {
        Ok(Flags {
            goal: self.merge.map(|merge| {
                if merge {
                    Goal::Merged
                } else {
                    Goal::ReadyForReview
                }
            }),
            email: self.email.as_ref().map(Email::ask).transpose()?,
            base_fix: self.base_fix.map(|fix| {
                if fix {
                    BaseFixAsk::Allow
                } else {
                    BaseFixAsk::Forbid
                }
            }),
            security_fix: self.security_fix.map(|fix| {
                if fix {
                    crate::security::FixAsk::Allow
                } else {
                    crate::security::FixAsk::Forbid
                }
            }),
            security_review: self.security_review.map(|review| {
                if review {
                    crate::security::review::Ask::Allow
                } else {
                    crate::security::review::Ask::Forbid
                }
            }),
            harness: harness::Asked {
                harness: self
                    .harness
                    .as_deref()
                    .map(|name| {
                        Harness::named(name).with_context(|| format!("unknown Harness {name}"))
                    })
                    .transpose()?,
                model_and_effort: ModelAndEffort {
                    model: self.model.clone(),
                    effort: self.effort.clone(),
                },
            },
            ..Flags::default()
        })
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
