//! Requested work: CLI arguments, detached lifetime and observation.
use std::fs::OpenOptions;
use std::io::{Seek, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use axum::{Extension, Json, response::Response};
use serde::{Deserialize, Serialize};

use super::{api, auth::Viewer};
mod experiment;
use crate::logs::start_notice::EarlyOutcome;
use crate::{
    asks::Flags,
    base_fix::BaseFixAsk,
    config::UserConfig,
    harness::{self, Harness, ModelAndEffort},
    issue::{IssueInput, IssueUrl, Repo},
    notification::NotificationAsk,
    run::Goal,
};
pub(super) use experiment::properties as experiment_properties;

#[derive(Serialize)]
#[serde(untagged)]
enum Started {
    Command {
        version: u8,
        name: String,
    },
    Early {
        version: u8,
        #[serde(flatten)]
        outcome: EarlyOutcome,
    },
}

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
    #[serde(default)]
    command: Kind,
    issue: Option<Issue>,
    merge: Option<bool>,
    email: Option<Email>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    base_fix: Option<bool>,
    security_review: Option<bool>,
    security_fix: Option<bool>,
    security_close: Option<bool>,
    parallel: Option<std::num::NonZeroUsize>,
    base: Option<String>,
    focus: Option<String>,
    plan_only: Option<bool>,
    order: Option<Order>,
    pull_request: Option<String>,
    author: Option<String>,
    recommend_only: Option<bool>,
    name: Option<String>,
    report: Option<String>,
    read_machine: Option<bool>,
    allow_ready_for_agent: Option<bool>,
    experiment: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind {
    #[default]
    Ticket,
    Architect,
    Pickup,
    Secure,
    Review,
    Fix,
    Process,
    Skill,
    Score,
    Triage,
    Experiment,
}

#[derive(Deserialize)]
enum Order {
    #[serde(rename = "lowest-first")]
    LowestFirst,
    #[serde(rename = "highest-first")]
    HighestFirst,
}

impl Kind {
    pub(super) const ALL: [Self; 11] = [
        Self::Ticket,
        Self::Architect,
        Self::Pickup,
        Self::Secure,
        Self::Review,
        Self::Fix,
        Self::Process,
        Self::Skill,
        Self::Score,
        Self::Triage,
        Self::Experiment,
    ];

    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Ticket => "ticket",
            Self::Architect => "architect",
            Self::Pickup => "pickup",
            Self::Secure => "secure",
            Self::Review => "review",
            Self::Fix => "fix",
            Self::Process => "process",
            Self::Skill => "skill",
            Self::Score => "score",
            Self::Triage => "triage",
            Self::Experiment => "experiment",
        }
    }

    /// One command-option inventory shared by admission and OpenAPI. The CLI
    /// parser still decides allowed values and combinations.
    pub(super) fn fields(self) -> impl Iterator<Item = &'static str> {
        let own: &[&str] = match self {
            Self::Ticket => &["issue"],
            Self::Architect => &["base", "focus", "plan_only"],
            Self::Pickup => &["base", "order"],
            Self::Secure => &["base"],
            Self::Review => &["pull_request", "author", "recommend_only"],
            Self::Fix => &["pull_request", "base", "author"],
            Self::Process => &["name", "issue", "pull_request", "plan_only"],
            Self::Skill => &["name", "issue"],
            Self::Score => &[],
            Self::Experiment => &["experiment"],
            Self::Triage => &[
                "report",
                "read_machine",
                "recommend_only",
                "allow_ready_for_agent",
            ],
        };
        let flags: &[&str] = match self {
            Self::Ticket | Self::Architect | Self::Pickup | Self::Secure => &[
                "merge",
                "email",
                "harness",
                "model",
                "effort",
                "base_fix",
                "security_review",
                "security_fix",
                "security_close",
                "parallel",
            ],
            Self::Review => &["email", "harness", "model", "effort", "security_review"],
            Self::Fix => &["merge", "email", "harness", "model", "effort", "base_fix"],
            Self::Process => &[
                "merge",
                "email",
                "harness",
                "model",
                "effort",
                "security_review",
                "security_fix",
                "security_close",
            ],
            Self::Triage => &["email", "harness", "model", "effort"],
            Self::Skill | Self::Score | Self::Experiment => &[],
        };
        own.iter().chain(flags).copied()
    }

    pub(super) fn required(self) -> &'static [&'static str] {
        match self {
            Self::Ticket => &["issue"],
            Self::Process | Self::Skill => &["name"],
            Self::Triage => &["report"],
            Self::Architect
            | Self::Pickup
            | Self::Secure
            | Self::Review
            | Self::Fix
            | Self::Score
            | Self::Experiment => &[],
        }
    }
}

impl Request {
    pub(super) fn arguments(&self) -> Result<(Repo, Vec<String>)> {
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
        self.check_fields()?;
        let issue = self.issue_url(&repo)?;
        let mut args = if self.command == Kind::Ticket {
            vec![issue.clone().context("issue is required")?]
        } else {
            vec![self.command.word().into()]
        };
        if matches!(self.command, Kind::Process | Kind::Skill) {
            args.push(self.name.clone().context("name is required")?);
            if let Some(issue) = issue {
                args.extend(["issue".into(), issue]);
            }
        }
        if self.command == Kind::Experiment {
            args.extend(experiment::arguments(self.experiment.as_ref())?);
        }
        if let Some(url) = &self.pull_request {
            let pr = crate::review::PullRequestUrl::parse(url)?;
            ensure!(
                pr.issue.repo().slug().eq_ignore_ascii_case(&repo.slug()),
                "pull request must be in {}",
                repo.slug()
            );
            if self.command == Kind::Process {
                args.push("pull-request".into());
            }
            args.push(url.clone());
        }
        if let Some(author) = &self.author {
            args.extend(["--author".into(), author.clone()]);
        }
        if let Some(base) = &self.base {
            args.extend(["--base".into(), base.clone()]);
        }
        if let Some(focus) = &self.focus {
            args.push(focus.clone());
        }
        if let Some(order) = &self.order {
            args.push(
                match order {
                    Order::LowestFirst => "--lowest-first",
                    Order::HighestFirst => "--highest-first",
                }
                .into(),
            );
        }
        for (enabled, word) in [
            (self.plan_only, "--plan-only"),
            (self.recommend_only, "--recommend-only"),
            (self.read_machine, "--read-machine"),
            (self.allow_ready_for_agent, "--allow-ready-for-agent"),
        ] {
            if enabled == Some(true) {
                args.push(word.into());
            }
        }
        crate::args::public_flag_words(&self.flags()?, &mut args);
        // The CLI decides allowed options and combinations. Also verify positional
        // JSON values stayed positional rather than becoming a CLI flag/subcommand.
        let parsed = crate::args::parse(&args)?;
        if let crate::args::Command::Architect(parsed) = &parsed {
            ensure!(
                parsed.focus == self.focus,
                "focus must be literal focus text"
            );
        }
        if self.command == Kind::Process {
            super::commands::process_name(self.name.as_deref().context("name is required")?)?;
            let crate::args::Command::Process(crate::args::ProcessAction::Run(parsed)) = parsed
            else {
                bail!("only a Process run can be started");
            };
            ensure!(
                Some(&parsed.name) == self.name.as_ref(),
                "name must be a Process name"
            );
        }
        Ok((repo, args))
    }

    fn check_fields(&self) -> Result<()> {
        let kind = self.command;
        for (present, field) in [
            (self.issue.is_some(), "issue"),
            (self.name.is_some(), "name"),
            (self.pull_request.is_some(), "pull_request"),
            (self.author.is_some(), "author"),
            (self.base.is_some(), "base"),
            (self.focus.is_some(), "focus"),
            (self.plan_only.is_some(), "plan_only"),
            (self.order.is_some(), "order"),
            (self.recommend_only.is_some(), "recommend_only"),
            (self.report.is_some(), "report"),
            (self.read_machine.is_some(), "read_machine"),
            (
                self.allow_ready_for_agent.is_some(),
                "allow_ready_for_agent",
            ),
            (self.merge.is_some(), "merge"),
            (self.email.is_some(), "email"),
            (self.harness.is_some(), "harness"),
            (self.model.is_some(), "model"),
            (self.effort.is_some(), "effort"),
            (self.base_fix.is_some(), "base_fix"),
            (self.security_review.is_some(), "security_review"),
            (self.security_fix.is_some(), "security_fix"),
            (self.security_close.is_some(), "security_close"),
            (self.parallel.is_some(), "parallel"),
            (self.experiment.is_some(), "experiment"),
        ] {
            ensure!(
                !present || kind.fields().any(|allowed| allowed == field),
                "{} takes no {field}",
                kind.word()
            );
        }
        if kind == Kind::Triage {
            ensure!(
                self.report
                    .as_ref()
                    .is_some_and(|text| !text.trim().is_empty()),
                "report text is required"
            );
        }
        Ok(())
    }

    fn issue_url(&self, repo: &Repo) -> Result<Option<String>> {
        let Some(issue) = &self.issue else {
            return Ok(None);
        };
        let input = match issue {
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
        Ok(Some(issue.url))
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
            parallel: self.parallel,
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
            security_close: self.security_close.map(|close| {
                if close {
                    crate::security::CloseAsk::Allow
                } else {
                    crate::security::CloseAsk::Forbid
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
        })
    }
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    Json(request): Json<Request>,
) -> Response {
    api::action(api::Format::Json, "/", move || start(request, viewer)).await
}

pub(super) fn cli(value: serde_json::Value) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(start(
        serde_json::from_value(value)?,
        Viewer::local_cli(),
    )?)?)
}

pub(super) async fn check_process(Json(mut request): Json<Request>) -> Response {
    api::action(api::Format::Json, "/", move || {
        ensure!(
            matches!(request.command, Kind::Ticket | Kind::Process),
            "this endpoint checks a Process"
        );
        request.command = Kind::Process;
        let (repo, mut args) = request.arguments()?;
        args.insert(1, "check".into());
        let directory = super::commands::directory(&repo.slug())?;
        let mut command = Command::new(std::env::current_exe()?);
        command.args(args).current_dir(directory.path);
        let output = crate::process::output_until(
            &mut command,
            None,
            crate::process::Control {
                name: "Process check",
                interruption: crate::process::Interruption::Ordinary,
                stop: &|child| crate::process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
            },
            Instant::now() + Duration::from_secs(30),
            &|| Ok(()),
        )?;
        ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    })
    .await
}

fn start(request: Request, viewer: Viewer) -> Result<Started> {
    let (repo, args) = request.arguments()?;
    let directory = super::commands::directory(&repo.slug())?;
    // Records follow the exact target URL sent to the CLI, including repository
    // spelling that differs from the origin's case.
    let record_repo = if let Some(url) = &request.pull_request {
        crate::review::PullRequestUrl::parse(url)?.issue.repo()
    } else {
        match request.command {
            Kind::Ticket => IssueUrl::parse(&args[0])?.repo(),
            Kind::Score | Kind::Experiment => crate::scoring::repository(),
            Kind::Architect
            | Kind::Pickup
            | Kind::Secure
            | Kind::Review
            | Kind::Fix
            | Kind::Process
            | Kind::Skill
            | Kind::Triage => {
                Repo::of_origin(&format!("https://github.com/{}", directory.repository))
                    .context("known Launch directory must name a GitHub repository")?
            }
        }
    };
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
    let notice = tempfile::NamedTempFile::new()?;
    let stdin = if let Some(report) = &request.report {
        let mut file = tempfile::tempfile()?;
        file.write_all(report.as_bytes())?;
        file.rewind()?;
        Stdio::from(file)
    } else {
        Stdio::null()
    };
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
        .env(crate::logs::start_notice::ENV, notice.path())
        .stdin(stdin)
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
    let result = wait_for_record(
        &mut child,
        &config,
        &record_repo,
        &log_path,
        started_after,
        notice.path(),
    );
    // Dropping std::process::Child does not reap it. The Console may outlive it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    result
}

fn wait_for_record(
    child: &mut Child,
    config: &UserConfig,
    repo: &Repo,
    log: &std::path::Path,
    started_after: chrono::DateTime<chrono::Local>,
    notice: &std::path::Path,
) -> Result<Started> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(outcome @ EarlyOutcome::Started { .. }) =
            crate::logs::start_notice::read(notice, child.id())?
        {
            return Ok(Started::Early {
                version: 1,
                outcome,
            });
        }
        if let Some(name) =
            crate::logs::command_for_process(config, repo, child.id(), started_after)?
        {
            return Ok(Started::Command { version: 1, name });
        }
        if let Some(status) = child.try_wait()? {
            // A very short run can publish and finish between the first read and exit.
            if let Some(name) =
                crate::logs::command_for_process(config, repo, child.id(), started_after)?
            {
                return Ok(Started::Command { version: 1, name });
            }
            if status.success()
                && let Some(skipped) = crate::logs::start_notice::read(notice, child.id())?
            {
                return Ok(Started::Early {
                    version: 1,
                    outcome: skipped,
                });
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
