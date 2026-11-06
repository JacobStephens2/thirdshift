//! Page choices for the existing structured start API.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    args, asks::Flags, config::UserConfig, harness::Harness, notification::NotificationAsk,
};

use super::start::Request;

#[derive(Serialize, Deserialize)]
pub(super) struct Repository {
    pub repository: String,
    pub machine: String,
    pub label: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Form {
    pub repositories: Vec<Repository>,
    pub harness: String,
    pub harnesses: Vec<String>,
    pub error: Option<String>,
}

impl Form {
    pub fn load(config: &UserConfig) -> Self {
        let mut form = Self {
            repositories: Vec::new(),
            harness: config
                .harness
                .default
                .unwrap_or(Harness::Claude)
                .name()
                .into(),
            harnesses: Harness::ALL
                .into_iter()
                .map(|harness| harness.name().to_owned())
                .collect(),
            error: None,
        };
        match crate::launch_directories::load() {
            Ok(inventory) => {
                let mut repositories: Vec<_> = inventory
                    .directories
                    .into_iter()
                    .map(|d| d.repository)
                    .collect();
                repositories.sort();
                repositories.dedup();
                form.repositories = repositories
                    .into_iter()
                    .map(|repository| Repository {
                        label: format!(
                            "{repository} · {}",
                            super::fleet_control::local_name(config)
                        ),
                        repository,
                        machine: String::new(),
                    })
                    .collect();
            }
            Err(error) => form.error = Some(error.to_string()),
        }
        form
    }

    pub fn fleet(
        config: &UserConfig,
        viewer: &super::auth::Viewer,
        targets: &[super::fleet_control::Target],
    ) -> Option<Self> {
        if !targets.iter().any(|target| target.run) {
            return None;
        }
        let mut form = Self::load(config);
        if !viewer.scopes.contains(&super::Scope::Run) {
            form.repositories.clear();
        }
        for target in targets
            .iter()
            .filter(|target| target.run && !target.machine.is_empty())
        {
            let result = super::fleet::read::Remote::select(
                config,
                &target.machine,
                viewer.fleet_person().map(str::to_owned),
            )
            .and_then(|remote| remote.unwrap().get("/start-options"))
            .and_then(|value| Ok(serde_json::from_value::<Options>(value)?));
            match result {
                Ok(options) => form
                    .repositories
                    .extend(options.form.repositories.into_iter().map(|mut repository| {
                        repository.label = format!("{} · {}", repository.repository, target.name);
                        repository.machine = target.machine.clone();
                        repository
                    })),
                Err(error) => form.error = Some(error.to_string()),
            }
        }
        Some(form)
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Pass {
    pub command: String,
    pub request: String,
    pub harness: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Options {
    pub form: Form,
    pub passes: Vec<Pass>,
    pub inventory: crate::cronjobs::CronJobs,
}

impl Options {
    pub fn load(config: &UserConfig) -> Result<Self> {
        let inventory = match crate::cronjobs::load() {
            Ok(inventory) => inventory,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                crate::cronjobs::CronJobs {
                    jobs: Vec::new(),
                    pass_starts: crate::logs::pass_starts::load(&config.logs_dir)?,
                }
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            form: Form::load(config),
            passes: passes(config, &inventory.jobs)?,
            inventory,
        })
    }
}

pub(super) async fn options() -> axum::response::Response {
    super::api::read(super::api::Format::Json, || {
        use axum::response::IntoResponse;
        Ok(axum::Json(Options::load(&UserConfig::load()?)?).into_response())
    })
    .await
}

/// Preserve the cron Pass's CLI options, while the API still decides admission.
pub(super) fn passes(config: &UserConfig, jobs: &[crate::cronjobs::CronJob]) -> Result<Vec<Pass>> {
    let directories = crate::launch_directories::load()?;
    let mut passes = Vec::new();
    for job in jobs {
        let Some((path, arguments)) = crate::launch_directories::cron_arguments(&job.command)
        else {
            continue;
        };
        let Some(directory) = directories.directories.iter().find(|d| d.path == path) else {
            continue;
        };
        let Ok(parsed) = args::parse(&arguments) else {
            continue;
        };
        let (mut request, flags, security) = match parsed {
            args::Command::Pickup(pass) => (
                json!({"command":"pickup", "base":pass.base, "order":pass.order.map(|o| match o {
                    crate::ready::PickupOrder::LowestFirst => "lowest-first",
                    crate::ready::PickupOrder::HighestFirst => "highest-first",
                })}),
                pass.flags,
                false,
            ),
            args::Command::Secure(pass) => (
                json!({"command":"secure", "base":pass.base}),
                pass.flags,
                true,
            ),
            args::Command::Architect(pass) => (
                json!({"command":"architect", "base":pass.base, "focus":pass.focus, "plan_only":pass.plan_only}),
                pass.flags,
                false,
            ),
            args::Command::Review(review) => {
                let crate::review::Target::Author(author) = review.target else {
                    continue;
                };
                (
                    json!({"command":"review", "author":author, "recommend_only":review.mode == crate::review::Mode::RecommendOnly}),
                    review.flags,
                    false,
                )
            }
            args::Command::Fix(fix) => {
                let crate::fix::Target::Pass { base, author } = fix.target else {
                    continue;
                };
                (
                    json!({"command":"fix", "base":base, "author":author}),
                    fix.flags,
                    false,
                )
            }
            args::Command::Process(args::ProcessAction::Run(process))
                if process.issue.is_none() && process.pull_request.is_none() =>
            {
                (
                    json!({"command":"process", "name":process.name, "plan_only":process.plan_only}),
                    Flags {
                        goal: process.goal,
                        email: process.email,
                        harness: process.asked,
                        security_review: process.security_review,
                        security_fix: process.security_fix,
                        security_close: process.security_close,
                        ..Flags::default()
                    },
                    false,
                )
            }
            args::Command::Help
            | args::Command::Version
            | args::Command::Update { .. }
            | args::Command::Setup
            | args::Command::Config { .. }
            | args::Command::Score
            | args::Command::Scores { .. }
            | args::Command::BugOrigins(_)
            | args::Command::Evals(_)
            | args::Command::Experiment(_)
            | args::Command::Experiments(_)
            | args::Command::Recheck(_)
            | args::Command::CronJobs { .. }
            | args::Command::LaunchDirectories { .. }
            | args::Command::Resources { .. }
            | args::Command::Console { .. }
            | args::Command::ConsoleInstall { .. }
            | args::Command::ConsolePassword(_)
            | args::Command::ConsoleSignOut { .. }
            | args::Command::ConsoleLog { .. }
            | args::Command::ConsoleToken(_)
            | args::Command::ConsoleFleet(_)
            | args::Command::ConsoleScopes(_)
            | args::Command::ConsoleWhoami { .. }
            | args::Command::ConsoleOpenApi
            | args::Command::ConsoleTheme { .. }
            | args::Command::ConsoleThemeImport(_)
            | args::Command::ConsoleOtherAccounts { .. }
            | args::Command::ConsoleThemeExport(_)
            | args::Command::ConsoleSections { .. }
            | args::Command::Status { .. }
            | args::Command::Show { .. }
            | args::Command::RemoteDetails { .. }
            | args::Command::FleetControl(_)
            | args::Command::Metrics(_)
            | args::Command::Spans { .. }
            | args::Command::Stop { .. }
            | args::Command::Dismiss { .. }
            | args::Command::Logs { .. }
            | args::Command::LogsList { .. }
            | args::Command::LogsExport { .. }
            | args::Command::LogFile { .. }
            | args::Command::ReleaseSummary
            | args::Command::Skill { .. }
            | args::Command::Process(_)
            | args::Command::Trial(_)
            | args::Command::OomExec { .. }
            | args::Command::EmailTest(_)
            | args::Command::Triage(_)
            | args::Command::Run(_) => continue,
        };
        request["repository"] = json!(directory.repository);
        add_flags(&mut request, &flags);
        // The same parser used at submission rejects unsupported combinations.
        let Ok(request_value) = serde_json::from_value::<Request>(request.clone()) else {
            continue;
        };
        if request_value.arguments().is_err() {
            continue;
        }
        let harness = if security {
            flags.security_harness(config)
        } else {
            flags.harness(config)
        };
        passes.push(Pass {
            command: job.command.clone(),
            request: serde_json::to_string(&request)?,
            harness: harness.harness.name().into(),
        });
    }
    Ok(passes)
}

pub(super) fn add_flags(request: &mut Value, flags: &Flags) {
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            request[key] = value;
        }
    };
    put(
        "merge",
        flags.goal.map(|g| json!(g == crate::run::Goal::Merged)),
    );
    put(
        "email",
        flags.email.as_ref().map(|email| match email {
            NotificationAsk::Send(address) => json!({"mode":"always", "address":address}),
            NotificationAsk::SendUnlessMerged(address) => {
                json!({"mode":"unless_merged", "address":address})
            }
            NotificationAsk::Skip => json!({"mode":"never"}),
        }),
    );
    put("parallel", flags.parallel.map(|n| json!(n.get())));
    put(
        "base_fix",
        flags
            .base_fix
            .as_ref()
            .map(|ask| json!(matches!(ask, crate::base_fix::BaseFixAsk::Allow))),
    );
    put(
        "security_review",
        flags
            .security_review
            .map(|ask| json!(ask == crate::security::review::Ask::Allow)),
    );
    put(
        "security_fix",
        flags
            .security_fix
            .map(|ask| json!(ask == crate::security::FixAsk::Allow)),
    );
    put(
        "security_close",
        flags
            .security_close
            .map(|ask| json!(ask == crate::security::CloseAsk::Allow)),
    );
    put("harness", flags.harness.harness.map(|h| json!(h.name())));
    put(
        "model",
        flags
            .harness
            .model_and_effort
            .model
            .as_ref()
            .map(|m| json!(m)),
    );
    put(
        "effort",
        flags
            .harness
            .model_and_effort
            .effort
            .as_ref()
            .map(|e| json!(e)),
    );
}
